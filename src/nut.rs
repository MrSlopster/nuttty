//! Minimal client for the NUT (upsd) line protocol over TCP.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Connect with an explicit timeout: a plain `TcpStream::connect` to an
/// unreachable (rather than refused) host blocks for the OS default of a
/// minute or more, which would freeze the single polling thread.
fn connect(host: &str, port: u16, io_timeout: Duration) -> Result<TcpStream> {
    let addrs = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("resolve {host}:{port}"))?;
    let mut last: Option<std::io::Error> = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => {
                s.set_read_timeout(Some(io_timeout))?;
                s.set_write_timeout(Some(io_timeout))?;
                return Ok(s);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(e) => anyhow!("connect {host}:{port}: {e}"),
        None => anyhow!("connect {host}:{port}: hostname resolved to no addresses"),
    })
}

pub struct NutClient {
    host: String,
    port: u16,
    stream: Option<BufReader<TcpStream>>,
}

impl NutClient {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
            stream: None,
        }
    }

    fn ensure(&mut self) -> Result<&mut BufReader<TcpStream>> {
        match &mut self.stream {
            Some(r) => Ok(r),
            slot => {
                let s = connect(&self.host, self.port, Duration::from_secs(3))?;
                Ok(slot.insert(BufReader::new(s)))
            }
        }
    }

    fn request_list(&mut self, what: &str, arg: &str) -> Result<Vec<String>> {
        let res = (|| {
            let r = self.ensure()?;
            let cmd = if arg.is_empty() {
                format!("LIST {what}\n")
            } else {
                format!("LIST {what} {arg}\n")
            };
            r.get_mut().write_all(cmd.as_bytes())?;
            let mut out = Vec::new();
            loop {
                let mut line = String::new();
                if r.read_line(&mut line)? == 0 {
                    bail!("connection closed by upsd");
                }
                let line = line.trim_end();
                if line.starts_with("BEGIN ") {
                    continue;
                }
                if line.starts_with("END ") {
                    break;
                }
                if let Some(err) = line.strip_prefix("ERR ") {
                    bail!("upsd: {err}");
                }
                out.push(line.to_string());
            }
            Ok(out)
        })();
        // Drop the connection on any error so the next poll reconnects cleanly.
        if res.is_err() {
            self.stream = None;
        }
        res
    }

    /// Returns (name, description) of every UPS the server knows.
    pub fn list_ups(&mut self) -> Result<Vec<(String, String)>> {
        let mut v = Vec::new();
        for l in self.request_list("UPS", "")? {
            if let Some(rest) = l.strip_prefix("UPS ") {
                // Tolerate a missing description field rather than dropping the UPS.
                let (name, desc) = rest.split_once(' ').unwrap_or((rest, ""));
                v.push((name.to_string(), unquote(desc)));
            }
        }
        Ok(v)
    }

    /// All instant commands the device supports, with their descriptions:
    /// `LIST CMD` for the names, then `GET CMDDESC` for each.
    pub fn list_cmds(&mut self, ups: &str) -> Result<Vec<(String, String)>> {
        let mut names = Vec::new();
        for l in self.request_list("CMD", ups)? {
            // CMD <ups> <name>
            if let Some(name) = l.split(' ').nth(2) {
                names.push(name.to_string());
            }
        }
        let mut v = Vec::with_capacity(names.len());
        for name in names {
            let desc = self.cmd_desc(ups, &name).unwrap_or_default();
            v.push((name, desc));
        }
        Ok(v)
    }

    fn cmd_desc(&mut self, ups: &str, cmd: &str) -> Result<String> {
        let res = (|| {
            let r = self.ensure()?;
            r.get_mut()
                .write_all(format!("GET CMDDESC {ups} {cmd}\n").as_bytes())?;
            let mut line = String::new();
            if r.read_line(&mut line)? == 0 {
                bail!("connection closed by upsd");
            }
            // CMDDESC <ups> <cmd> "<desc>"
            let desc = line
                .trim_end()
                .splitn(4, ' ')
                .nth(3)
                .map(unquote)
                .unwrap_or_default();
            // upsd uses "Unavailable" when the driver has no description.
            Ok(if desc == "Unavailable" {
                String::new()
            } else {
                desc
            })
        })();
        if res.is_err() {
            self.stream = None;
        }
        res
    }

    pub fn list_vars(&mut self, ups: &str) -> Result<BTreeMap<String, String>> {
        let mut map = BTreeMap::new();
        for l in self.request_list("VAR", ups)? {
            if let Some((name, val)) = parse_tagged_line(&l, "VAR") {
                map.insert(name, val);
            }
        }
        Ok(map)
    }

    /// Names of the variables the device reports as writable (`LIST RW`).
    pub fn list_rw(&mut self, ups: &str) -> Result<Vec<String>> {
        Ok(self
            .request_list("RW", ups)?
            .iter()
            .filter_map(|l| parse_tagged_line(l, "RW"))
            .map(|(name, _)| name)
            .collect())
    }
}

/// Parse a `<TAG> <ups> <name> "<value>"` response line (TAG is VAR or RW).
fn parse_tagged_line(l: &str, tag: &str) -> Option<(String, String)> {
    let mut it = l.splitn(4, ' ');
    if it.next() != Some(tag) {
        return None;
    }
    it.next(); // ups name
    let name = it.next()?;
    let val = it.next()?;
    Some((name.to_string(), unquote(val)))
}

/// Send one authenticated request on its own short-lived connection, so the
/// polling connection never carries auth state.
fn authed_line(host: &str, port: u16, user: &str, pass: &str, line: &str) -> Result<String> {
    let s = connect(host, port, Duration::from_secs(5))?;
    let mut r = BufReader::new(s);
    let mut send = |line: String| -> Result<String> {
        r.get_mut().write_all(line.as_bytes())?;
        let mut resp = String::new();
        r.read_line(&mut resp)?;
        Ok(resp.trim_end().to_string())
    };
    let u = send(format!("USERNAME {user}\n"))?;
    if !u.starts_with("OK") {
        bail!("USERNAME: {u}");
    }
    let p = send(format!("PASSWORD {pass}\n"))?;
    if !p.starts_with("OK") {
        bail!("PASSWORD: {p}");
    }
    send(line.to_string())
}

/// Run an instant command.
pub fn instcmd(
    host: &str,
    port: u16,
    ups: &str,
    cmd: &str,
    user: &str,
    pass: &str,
) -> Result<String> {
    authed_line(host, port, user, pass, &format!("INSTCMD {ups} {cmd}\n"))
}

/// Set a read-write variable. Requires a NUT user with `actions = SET`.
pub fn set_var(
    host: &str,
    port: u16,
    ups: &str,
    name: &str,
    value: &str,
    user: &str,
    pass: &str,
) -> Result<String> {
    authed_line(host, port, user, pass, &set_var_line(ups, name, value))
}

/// Escape a value for the wire: the exact inverse of `unquote`.
fn quote_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn set_var_line(ups: &str, name: &str, value: &str) -> String {
    format!("SET VAR {ups} {name} \"{}\"\n", quote_value(value))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(s);
    let mut out = String::with_capacity(inner.len());
    let mut esc = false;
    for c in inner.chars() {
        if esc {
            out.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquote_plain_and_quoted() {
        assert_eq!(unquote("plain"), "plain");
        assert_eq!(unquote("\"quoted value\""), "quoted value");
        assert_eq!(unquote("  \"padded\"  "), "padded");
    }

    #[test]
    fn unquote_escapes() {
        assert_eq!(unquote(r#""a \"b\" c""#), r#"a "b" c"#);
        assert_eq!(unquote(r#""back\\slash""#), r"back\slash");
    }

    #[test]
    fn unquote_unbalanced_quote_kept() {
        // A lone opening quote is not a quoted value; keep it verbatim.
        assert_eq!(unquote("\"dangling"), "\"dangling");
    }

    #[test]
    fn var_line_parses() {
        assert_eq!(
            parse_tagged_line(r#"VAR myups battery.charge "95""#, "VAR"),
            Some(("battery.charge".into(), "95".into()))
        );
        assert_eq!(
            parse_tagged_line(r#"VAR myups ups.status "OL CHRG""#, "VAR"),
            Some(("ups.status".into(), "OL CHRG".into()))
        );
    }

    #[test]
    fn rw_line_parses() {
        assert_eq!(
            parse_tagged_line(r#"RW myups input.transfer.low "160""#, "RW"),
            Some(("input.transfer.low".into(), "160".into()))
        );
        // Tag mismatch: a VAR line is not an RW line.
        assert_eq!(parse_tagged_line(r#"VAR myups x "1""#, "RW"), None);
    }

    #[test]
    fn set_var_wire_format() {
        assert_eq!(
            set_var_line("myups", "input.transfer.low", "1.5"),
            "SET VAR myups input.transfer.low \"1.5\"\n"
        );
        // Quotes and backslashes in the value must be escaped on the wire.
        assert_eq!(
            set_var_line("u", "n", r#"a"b\c"#),
            "SET VAR u n \"a\\\"b\\\\c\"\n"
        );
        // quote_value is the exact inverse of unquote.
        let tricky = r#"a"b\c \" end"#;
        assert_eq!(unquote(&format!("\"{}\"", quote_value(tricky))), tricky);
    }

    #[test]
    fn var_line_rejects_malformed() {
        assert_eq!(parse_tagged_line("CMD myups beeper.enable", "VAR"), None);
        assert_eq!(parse_tagged_line("VAR myups", "VAR"), None);
        assert_eq!(parse_tagged_line("", "VAR"), None);
    }
}
