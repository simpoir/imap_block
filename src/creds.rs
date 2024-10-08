use async_std::fs::File;
use async_std::path::Path;
use async_std::prelude::*;

use crate::errors::Res;

pub struct Creds {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
}

impl Creds {
    pub fn from_stdin() -> Res<Creds> {
        log::debug!("waiting from creds on stdin");
        let mut host = String::new();
        let mut port = 993;
        let mut user = String::new();
        let mut pass = String::new();
        std::io::stdin().read_line(&mut pass)?;
        pass = pass.trim().into();
        let mut line = String::new();
        while let Ok(siz) = std::io::stdin().read_line(&mut line) {
            if siz == 0 {
                break;
            }
            if let Some(stripped) = line.strip_prefix("user:") {
                user = stripped.into();
            } else if let Some(stripped) = line.strip_prefix("imap:") {
                host = stripped.into();
                if let Some((head, tail)) = host.split_once(':') {
                    port = tail.parse()?;
                    host = head.into();
                }
            }
            line.clear();
        }
        Ok(Creds {
            host,
            user,
            pass,
            port,
        })
    }

    pub async fn from_mutt(conf: &Path) -> Res<Creds> {
        let mut c = String::new();
        File::open(conf).await?.read_to_string(&mut c).await?;

        let mut host = String::new();
        let mut port = 993;
        let mut user = String::new();
        let mut pass = String::new();
        for l in c.lines() {
            if l.contains("imap_pass") {
                if let Some(sep) = l.find('=') {
                    let (_, v) = l.split_at(sep + 1);
                    pass = v.trim().trim_matches('\'').trim_matches('"').into();
                    if pass.starts_with('`') {
                        let cmd = std::process::Command::new("/bin/sh")
                            .arg("-c")
                            .arg(pass.trim_matches('`'))
                            .output()?;
                        pass = String::from_utf8_lossy(&cmd.stdout)
                            .lines()
                            .next()
                            .unwrap()
                            .to_string();
                    }
                };
            }

            if l.contains("imap_user") {
                if let Some(sep) = l.find('=') {
                    let (_, v) = l.split_at(sep + 1);
                    user = v.trim().trim_matches('\'').trim_matches('"').into();
                };
            }

            if l.contains("folder") {
                if let Some(sep) = l.find('=') {
                    let (_, v) = l.split_at(sep + 1);
                    let raw_url = v.trim().trim_matches('\'').trim_matches('"');
                    let url = urlparse::urlparse(raw_url);
                    if let Some(h) = url.hostname {
                        host = h;
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

impl std::fmt::Debug for Creds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        // Simple debug format without leaking credentials.
        f.debug_struct("Creds")
            .field("host", &self.host)
            .field("port", &self.port)
            .finish()
    }
}

#[cfg(test)]
mod tests {

    use async_std::task::block_on;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_missing() {
        let tmp = NamedTempFile::new().unwrap();
        let path = String::from(tmp.path().to_string_lossy());
        drop(tmp);

        let c = block_on(super::Creds::from_mutt(&path));
        c.expect_err("file should not be found");
    }

    #[test]
    fn test_parse_empty() {
        let tmp = NamedTempFile::new().unwrap();
        let c = block_on(super::Creds::from_mutt(tmp.path().to_str().unwrap()));
        let c = c.unwrap();
        assert_eq!(993, c.port);
        assert_eq!("", c.host);
        assert_eq!("", c.user);
        assert_eq!("", c.pass);
    }

    #[test]
    fn test_parse() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(
            tmp,
            "{}",
            textwrap::dedent(
                "
                set imap_user = 'my_user'
                set imap_pass = \"my_pass\"
                set folder    = imaps://host.name:123/
                "
            )
        )
        .unwrap();
        let c = block_on(super::Creds::from_mutt(tmp.path().to_str().unwrap()));
        let c = c.unwrap();
        assert_eq!(123, c.port);
        assert_eq!("host.name", c.host);
        assert_eq!("my_user", c.user);
        assert_eq!("my_pass", c.pass);
    }
}
