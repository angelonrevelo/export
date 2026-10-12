//! In-memory job runner for a small Export server.
//!
//! One process, a hard byte budget, no threads, no model. Duplicate bytes
//! (same SHA-256) reuse the existing job instead of storing a second copy.

use std::fmt::Write as _;

use export_kernel::{sha256, sniff, write_box, Kind, Section};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Empty,
    OverBudget,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: u64,
    pub kind: Kind,
    pub hash: [u8; 32],
    pub boxed: Vec<u8>,
    pub payload_len: usize,
}

#[derive(Debug)]
pub struct Runner {
    budget: usize,
    used: usize,
    next_id: u64,
    jobs: Vec<Job>,
}

impl Runner {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            used: 0,
            next_id: 1,
            jobs: Vec::new(),
        }
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Store `bytes` if they fit. A repeated hash returns the original id and
    /// does not consume more of the budget.
    pub fn submit(&mut self, bytes: &[u8]) -> Result<u64, Error> {
        if bytes.is_empty() {
            return Err(Error::Empty);
        }
        let hash = sha256(bytes);
        if let Some(existing) = self.jobs.iter().find(|job| job.hash == hash) {
            return Ok(existing.id);
        }
        if self.used.saturating_add(bytes.len()) > self.budget {
            return Err(Error::OverBudget);
        }
        let kind = sniff(bytes);
        let boxed = write_box(&[Section {
            id: 1,
            bytes: bytes.to_vec(),
        }]);
        let id = self.next_id;
        self.next_id += 1;
        self.used += bytes.len();
        self.jobs.push(Job {
            id,
            kind,
            hash,
            boxed,
            payload_len: bytes.len(),
        });
        Ok(id)
    }

    pub fn get(&self, id: u64) -> Result<&Job, Error> {
        self.jobs.iter().find(|job| job.id == id).ok_or(Error::Missing)
    }

    /// Drop oldest jobs until `bytes` are free. Returns how many were dropped.
    pub fn reclaim(&mut self, bytes: usize) -> usize {
        let mut dropped = 0;
        while self.used + bytes > self.budget && !self.jobs.is_empty() {
            let job = self.jobs.remove(0);
            self.used -= job.payload_len;
            dropped += 1;
        }
        dropped
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// Handle one HTTP/1.1 request already split into method, path, and body.
pub fn handle(runner: &mut Runner, method: &str, path: &str, body: &[u8]) -> HttpResponse {
    match (method, path) {
        ("POST", "/v1/jobs") => match runner.submit(body) {
            Ok(id) => json_ok(&format!("{{\"id\":{id}}}")),
            Err(Error::Empty) => text(400, "empty"),
            Err(Error::OverBudget) => text(413, "over budget"),
            Err(Error::Missing) => text(404, "missing"),
        },
        ("GET", path) if path.starts_with("/v1/jobs/") => {
            let id = match path["/v1/jobs/".len()..].parse::<u64>() {
                Ok(id) => id,
                Err(_) => return text(400, "bad id"),
            };
            match runner.get(id) {
                Ok(job) => {
                    let mut body = String::new();
                    let _ = write!(
                        body,
                        "{{\"id\":{},\"kind\":\"{}\",\"bytes\":{}}}",
                        job.id,
                        kind_name(job.kind),
                        job.payload_len
                    );
                    json_ok(&body)
                }
                Err(Error::Missing) => text(404, "missing"),
                Err(_) => text(400, "bad request"),
            }
        }
        ("GET", path) if path.starts_with("/v1/files/") => {
            let id = match path["/v1/files/".len()..].parse::<u64>() {
                Ok(id) => id,
                Err(_) => return text(400, "bad id"),
            };
            match runner.get(id) {
                Ok(job) => HttpResponse {
                    status: 200,
                    content_type: "application/octet-stream",
                    body: job.boxed.clone(),
                },
                Err(Error::Missing) => text(404, "missing"),
                Err(_) => text(400, "bad request"),
            }
        }
        _ => text(404, "not found"),
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Jpeg => "jpeg",
        Kind::Png => "png",
        Kind::Gif => "gif",
        Kind::Webp => "webp",
        Kind::Wav => "wav",
        Kind::Mp4 => "mp4",
        Kind::Pdf => "pdf",
        Kind::Unknown => "unknown",
    }
}

fn json_ok(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        content_type: "application/json",
        body: body.as_bytes().to_vec(),
    }
}

fn text(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        content_type: "text/plain",
        body: body.as_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::{handle, Runner, Error};
    use export_kernel::{read_box, Kind};

    #[test]
    fn dedup_does_not_spend_the_budget_twice() {
        let mut runner = Runner::new(8);
        let id = runner.submit(b"12345678").unwrap();
        assert_eq!(runner.submit(b"12345678").unwrap(), id);
        assert_eq!(runner.used(), 8);
        assert_eq!(runner.len(), 1);
        assert_eq!(runner.submit(b"abcdefgh"), Err(Error::OverBudget));
    }

    #[test]
    fn sniff_and_box_round_trip() {
        let mut runner = Runner::new(64);
        let png = b"\x89PNG\r\n\x1a\nrest";
        let id = runner.submit(png).unwrap();
        let job = runner.get(id).unwrap();
        assert_eq!(job.kind, Kind::Png);
        let sections = read_box(&job.boxed).unwrap();
        assert_eq!(sections[0].bytes, png);
    }

    #[test]
    fn http_post_and_get() {
        let mut runner = Runner::new(32);
        let created = handle(&mut runner, "POST", "/v1/jobs", b"%PDF-1.4");
        assert_eq!(created.status, 200);
        assert_eq!(created.body, b"{\"id\":1}");
        let status = handle(&mut runner, "GET", "/v1/jobs/1", b"");
        assert_eq!(status.status, 200);
        assert_eq!(
            std::str::from_utf8(&status.body).unwrap(),
            "{\"id\":1,\"kind\":\"pdf\",\"bytes\":8}"
        );
        let file = handle(&mut runner, "GET", "/v1/files/1", b"");
        assert_eq!(file.status, 200);
        assert!(file.body.starts_with(b"EXPBOX01"));
        assert_eq!(handle(&mut runner, "GET", "/v1/jobs/9", b"").status, 404);
    }

    #[test]
    fn reclaim_drops_oldest() {
        let mut runner = Runner::new(4);
        runner.submit(b"abcd").unwrap();
        assert_eq!(runner.reclaim(4), 1);
        assert!(runner.is_empty());
        assert_eq!(runner.submit(b"wxyz").unwrap(), 2);
    }
}
