use std::{fs, io, path, thread, time::Duration};

use clap::{App, AppSettings, Arg};

use dogstatsd::{Client, Options};

use serde_json::{Map, Value};

trait Report {
    fn report(&mut self, tag: String, value: String) -> io::Result<()>;
}

struct Interface {
    name: String,
    alias: String,
    entries: Vec<Entry>,
}

struct Entry {
    path: path::PathBuf,
    tag: String,
}

struct DatadogReporter {
    client: Client,
}

struct LogReporter {}

impl LogReporter {
    pub fn new() -> Self {
        Self {}
    }
}

impl Interface {
    pub fn from_path(prefix: String, name: String, alias: String) -> io::Result<Self> {
        // What does this mean Kobe Bryant?
        let mut s = name.to_owned();
        s.pop().unwrap().to_string();
        s.remove(0).to_string();
        let root_dir = "/sys/class/net/".to_owned() + s.as_str() + "/statistics/";
        let entries = fs::read_dir(root_dir)?
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| Entry {
                path: e.path(),
                tag: prefix.to_owned() + alias.as_str() + "." + e.file_name().to_str().unwrap(),
            })
            .collect::<Vec<_>>();
        Ok(Self {
            name,
            alias,
            entries,
        })
    }

    pub fn report<R: Report>(&mut self, reporter: &mut R) -> Vec<io::Result<()>> {
        self.entries
            .iter()
            .map(|entry| {
                fs::read_to_string(entry.path.as_path())
                    .and_then(|mut contents| {
                        if contents.ends_with('\n') {
                            contents.pop();
                        }
                        Ok(contents)
                    })
                    .and_then(|contents| {
                        contents
                            .parse::<u64>()
                            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
                    })
                    .and_then(|value| {
                        reporter
                            .report(entry.tag.to_string(), value.to_string())
                            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
                    })
            })
            .collect::<Vec<_>>()
    }
}

impl DatadogReporter {
    pub fn new() -> io::Result<Self> {
        match Client::new(Options::default()) {
            Ok(client) => Ok(Self { client }),
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
        }
    }
}

impl Report for DatadogReporter {
    fn report(&mut self, tag: String, value: String) -> io::Result<()> {
        self.client
            .gauge(tag.to_string(), value.to_string(), &["tag:required"])
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

impl Report for LogReporter {
    fn report(&mut self, tag: String, value: String) -> io::Result<()> {
        println!("{} {}", tag, value);
        Ok(())
    }
}

fn create_report(filename: String, prefix: String) -> io::Result<Vec<Interface>> {
    let content = fs::read_to_string(filename)?;
    let json: Map<String, Value> = serde_json::from_str(&content)?;
    let ifaces = json
        .iter()
        .map(move |(alias, iface)| {
            match Interface::from_path(prefix.to_string(), iface.to_string(), alias.to_string()) {
                Ok(i) => {
                    println!("Watching {}", i.alias);
                    Ok(i)
                }
                Err(e) => {
                    println!("Slain by {:?} {:?}. Reason: {:?}", alias, iface, e);
                    Err(e)
                }
            }
        })
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    Ok(ifaces)
}

fn main() -> io::Result<()> {
    let matches = App::new("ethwatch")
        .setting(AppSettings::ArgRequiredElseHelp)
        .version("0.0.1")
        .author("Cameron Dart <cdart@anduril.com>")
        .about("Publishes /sys/class/net/<iface> metrics to datadog")
        .arg(Arg::with_name("filename").index(1).required(true).help(
            "json file containing a set of key-value pairs of clean name to interface to watch",
        ))
        .arg(
            Arg::with_name("prefix")
                .index(2)
                .required(true)
                .help("prefix to report metrics with."),
        )
        .arg(Arg::with_name("config"))
        .get_matches();

    let filename = matches.value_of("filename").expect("No file passed");
    let prefix = matches.value_of("prefix").expect("No prefix passed");
    let mut report = create_report(filename.to_string(), prefix.to_string())?;

    if report.len() == 0 {
        return Err(io::Error::new(io::ErrorKind::Other, "Empty report"));
    }

    let mut dd = DatadogReporter::new()?;
    // let mut dd = LogReporter::new();

    loop {
        for iface in report.iter_mut() {
            let _ = iface.report(&mut dd);
        }
        thread::sleep(Duration::from_millis(500));
    }
}
