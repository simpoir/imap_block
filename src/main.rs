use async_std::fs::File;
use async_std::prelude::*;
use async_std::task::sleep;
use log::{self, debug, error, warn};
use std::env::args;
use std::process::exit;
use std::time::Duration;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const POLL: u64 = 300;
const KEEP_ALIVE: u64 = 1700;

struct Backoff<'a> {
    i: usize,
    v: &'a [u64],
}

impl<'a> Backoff<'a> {
    pub fn new(v: &'a [u64]) -> Backoff<'a> {
        Backoff { i: 0, v }
    }

    pub fn next(&mut self) -> u64 {
        let ret = self.v[self.i];
        self.i = std::cmp::min(self.i + 1, self.v.len());
        ret
    }

    pub fn reset(&mut self) {
        self.i = 0;
    }
}

struct Creds {
    host: String,
    port: u16,
    user: String,
    pass: String,
}

impl Creds {
    pub async fn from_mutt(conf: String) -> Res<Creds> {
        let mut c = String::new();
        File::open(conf).await?.read_to_string(&mut c).await?;

        let mut host = String::new();
        let mut port = 993;
        let mut user = String::new();
        let mut pass = String::new();
        for l in c.lines() {
            if l.contains("imap_pass") {
                if let Some(sep) = l.find("=") {
                    let (_, v) = l.split_at(sep + 1);
                    pass = v.trim().trim_matches('\'').trim_matches('"').into();
                };
            }

            if l.contains("imap_user") {
                if let Some(sep) = l.find("=") {
                    let (_, v) = l.split_at(sep + 1);
                    user = v.trim().trim_matches('\'').trim_matches('"').into();
                };
            }

            if l.contains("folder") {
                if let Some(sep) = l.find("=") {
                    let (_, v) = l.split_at(sep + 1);
                    let raw_url = v.trim().trim_matches('\'').trim_matches('"');
                    let url = urlparse::urlparse(raw_url);
                    if let Some(h) = url.hostname {
                        host = h.into();
                    }
                    if let Some(p) = url.port {
                        port = p;
                    }
                }
            }
        }

        Ok(Creds {
            host,
            port,
            user,
            pass,
        })
    }
}

fn dump_status(count: usize) {
    let percent = if count > 0 { 100 } else { 0 };
    println!("{{\"text\": \"{}\", \"percentage\": {}}}", count, percent);
}

#[async_std::main]
async fn main() {
    // env RUST_LOG=debug
    env_logger::init();

    let mut argv = args();
    let prog = argv.next().unwrap();
    let conf_path = match argv.next() {
        Some(v) => v,
        None => {
            error!("Syntax: {} <mutt_conf>", prog);
            exit(1);
        }
    };

    let cred = match Creds::from_mutt(conf_path).await {
        Ok(v) => v,
        Err(e) => {
            error!("Problem reading config: {}", e);
            exit(1);
        }
    };

    let mut backoff = Backoff::new(&[0, 60, 120, 500, 600]);
    'retrying: loop {
        sleep(Duration::from_secs(backoff.next())).await;
        let tls = async_native_tls::TlsConnector::new();
        let host = cred.host.as_str();
        let c = match async_imap::connect((host, cred.port), host, tls).await {
            Ok(c) => c,
            Err(e) => {
                warn!("Error connecting: {}", e);
                continue 'retrying;
            }
        };
        let mut s = match c.login(cred.user.as_str(), cred.pass.as_str()).await {
            Ok(s) => s,
            Err((e, _)) => {
                error!("Failure logging in: {}", e);
                std::process::exit(2);
            }
        };
        debug!("logged in successfully");

        let can_idle = match s.capabilities().await {
            Ok(cap) => cap.has_str("IDLE"),
            Err(e) => {
                error!("Failure listing caps: {}", e);
                exit(2);
            }
        };
        debug!("Server can IDLE: {}", can_idle);

        match s.examine("INBOX").await {
            Ok(mb) => mb,
            Err(e) => {
                debug!("Failure listing mailbox: {}", e);
                continue 'retrying;
            }
        };

        'poll: loop {
            let count = match s.search("UNSEEN").await {
                Ok(ids) => ids.len(),
                Err(e) => {
                    debug!("Failure searching unread: {}", e);
                    continue 'retrying;
                }
            };

            dump_status(count);
            backoff.reset();

            if !can_idle {
                sleep(Duration::from_secs(POLL)).await;
                continue 'poll;
            }

            debug!("idling");
            let mut idle = s.idle();
            if let Err(e) = idle.init().await {
                debug!("Failed to idle: {}", e);
                continue 'retrying;
            };
            let (fut, stopper) = idle.wait_with_timeout(Duration::from_secs(KEEP_ALIVE));
            if let Err(e) = fut.await {
                debug!("Failed while idle: {}", e);
                continue 'retrying;
            };
            s = match idle.done().await {
                Ok(s) => s,
                Err(e) => {
                    debug!("Failed to end idle: {}", e);
                    continue 'retrying;
                }
            };
            drop(stopper); // drop only after waiting to avoid early return
            debug!("done idling");
        }
    }
}
