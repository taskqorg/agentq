//! Installation-owned SQLite telemetry; public entrypoints serialize through the existing lifecycle lock.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::path::Path;

const APP: i64 = 0x4151544d;
const PAGES: i64 = 7168;
const WINDOW: u64 = 7 * 86400;
const RECORD_MAX: usize = 64 << 10;

fn open(data: &Path) -> Result<Connection, String> {
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(data, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let path = data.join("telemetry.sqlite3");
    let exists = path.exists();
    let db = Connection::open(&path).map_err(|e| format!("local telemetry SQLite: {e}"))?;
    db.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    let (app, version): (i64, i64) = (
        db.query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(|e| e.to_string())?,
        db.query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?,
    );
    if exists && (app != APP || version != 1) {
        return Err("local telemetry SQLite: foreign or unsupported schema; file kept".into());
    }
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY; PRAGMA cache_spill=OFF; PRAGMA cache_size=-32768; PRAGMA wal_autocheckpoint=0;").map_err(|e| e.to_string())?;
    db.pragma_update(None, "max_page_count", PAGES)
        .map_err(|e| e.to_string())?;
    if !exists {
        db.execute_batch(&format!("BEGIN IMMEDIATE; PRAGMA application_id={APP}; PRAGMA user_version=1;
            CREATE TABLE events(seq INTEGER PRIMARY KEY, kind TEXT NOT NULL, id TEXT NOT NULL, time INTEGER NOT NULL, day TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(kind,id));
            CREATE INDEX events_time ON events(kind,time); CREATE INDEX events_day ON events(kind,day);
            CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            COMMIT;")).map_err(|e| e.to_string())?;
    }
    Ok(db)
}
fn checkpoint(db: &Connection) -> Result<(), String> {
    let busy: i64 = db
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if busy != 0 {
        return Err("local telemetry checkpoint busy; write refused".into());
    }
    Ok(())
}
fn raw_meta(db: &Connection, key: &str) -> Result<Option<String>, String> {
    db.query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())
}
fn set(db: &Connection, key: &str, value: &Value) -> Result<(), String> {
    if value.to_string().len() > RECORD_MAX {
        return Err("local telemetry metadata exceeds 64 KiB; originals kept".into());
    }
    db.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value.to_string()]).map_err(|e| e.to_string())?;
    Ok(())
}
fn events(db: &Connection, kind: &str) -> Result<Vec<Value>, String> {
    let mut s = db
        .prepare("SELECT payload FROM events WHERE kind=?1 ORDER BY seq")
        .map_err(|e| e.to_string())?;
    s.query_map([kind], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .map(|s| {
            serde_json::from_str(&s.map_err(|e| e.to_string())?)
                .map_err(|e| format!("telemetry event: {e}"))
        })
        .collect()
}
fn exists(db: &Connection, kind: &str, id: &str, v: &Value) -> Result<bool, String> {
    let previous: Option<String> = db
        .query_row(
            "SELECT payload FROM events WHERE kind=?1 AND id=?2",
            params![kind, id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(old) = previous {
        if serde_json::from_str::<Value>(&old).map_err(|e| e.to_string())? != *v {
            return Err("conflicting local telemetry event ID; history kept".into());
        }
        return Ok(true);
    }
    Ok(false)
}
fn insert(db: &Connection, kind: &str, id: &str, v: &Value) -> Result<bool, String> {
    let payload = v.to_string();
    if payload.len() > RECORD_MAX {
        return Err("local telemetry record exceeds 64 KiB".into());
    }
    if exists(db, kind, id, v)? {
        return Ok(false);
    }
    let at = v["time"]
        .as_u64()
        .filter(|t| *t <= i64::MAX as u64)
        .ok_or("telemetry missing or invalid time")?;
    db.execute(
        "INSERT INTO events(kind,id,time,day,payload) VALUES(?1,?2,?3,?4,?5)",
        params![kind, id, at as i64, crate::date(at), payload],
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}
#[cfg(test)]
fn counts(db: &Connection) -> Result<serde_json::Map<String, Value>, String> {
    let mut q = db
        .prepare("SELECT day,count(*) FROM events WHERE kind='stats' GROUP BY day")
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut out = serde_json::Map::new();
    for row in rows {
        let (day, n) = row.map_err(|e| e.to_string())?;
        out.insert(day, json!(n));
    }
    Ok(out)
}

fn maintain(db: &mut Connection, now: u64, bytes: usize) -> Result<(), String> {
    let reserve = (bytes as i64).div_euclid(4096) * 2 + 34;
    if reserve >= PAGES {
        return Err("local telemetry batch exceeds capacity; history kept".into());
    }
    loop {
        checkpoint(db)?;
        let pages: i64 = db
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let free: i64 = db
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let pressure = pages - free + reserve >= PAGES;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let rows = {
            let mut q=tx.prepare("SELECT seq,kind,day,time FROM events WHERE time < ?1 OR ?2 ORDER BY time,seq LIMIT 256").map_err(|e|e.to_string())?;
            q.query_map(
                params![
                    now.saturating_sub(WINDOW).min(i64::MAX as u64) as i64,
                    pressure
                ],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        };
        if rows.is_empty() {
            if pressure {
                return Err("local telemetry full; write refused".into());
            }
            return Ok(());
        }
        let mut journal = raw_meta(&tx, "journal")?
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .filter(crate::local::journal_valid);
        for (seq, kind, day, at) in rows {
            tx.execute("DELETE FROM events WHERE seq=?1", [seq])
                .map_err(|e| e.to_string())?;
            if kind == "stats" {
                if let Some(j) = journal.as_mut() {
                    j["counts"][&day] =
                        json!(j["counts"][&day].as_u64().unwrap_or(0).saturating_sub(1));
                    if pressure {
                        j["since"] =
                            json!(j["since"].as_u64().unwrap_or(0).max(at.max(0) as u64 + 1));
                    }
                }
            }
        }
        if let Some(j) = journal {
            set(&tx, "journal", &j)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
    }
}

/// Called with the lifecycle lock held. A record and its coverage commit in one transaction.
#[cfg(test)]
pub fn save_locked(data: &Path, r: &Value, failed: bool) -> Result<(), String> {
    crate::local::validate_record(r)?;
    if r.to_string().len() > RECORD_MAX {
        return Err("local telemetry record exceeds 64 KiB".into());
    }
    let mut db = open(data)?;
    maintain(&mut db, crate::now(), r.to_string().len())?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let at = r["time"].as_u64().ok_or("missing time")?;
    let held = counts(&tx)?;
    let journal = raw_meta(&tx, "journal")?.map(String::into_bytes);
    let mut j = crate::local::coverage(
        if failed {
            Some(b"")
        } else {
            journal.as_deref()
        },
        !held.is_empty(),
        &held,
        at,
        crate::now().max(at),
    );
    if insert(
        &tx,
        "stats",
        r["event_id"].as_str().ok_or("missing event ID")?,
        r,
    )? {
        let day = crate::date(at);
        j["counts"][&day] = json!(
            j["counts"][&day]
                .as_u64()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("coverage count overflow; history kept")?
        );
    }
    set(&tx, "journal", &j)?;
    tx.commit().map_err(|e| e.to_string())?;
    checkpoint(&db)
}

#[cfg(test)]
pub fn snapshot(data: &Path) -> Result<Value, String> {
    if !data.join("telemetry.sqlite3").exists() {
        return Err("stats unknown: no stats file".into());
    }
    let _lock = crate::executor::lock(data)?;
    let mut db = open(data)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let journal = raw_meta(&tx, "journal")?
        .map(|s| serde_json::from_str::<Value>(&s).unwrap_or(json!("unreadable")));
    Ok(
        json!({"records":events(&tx,"stats")?,"journal":journal,"held":counts(&tx)?}),
    )
}

fn trace_value(v: &Value) -> Result<Value, String> {
    let at = v["time"].as_u64().ok_or("traces unknown: malformed line")?;
    let op = v["op"].as_str().ok_or("traces unknown: malformed line")?;
    let step = v["step"].as_str().ok_or("traces unknown: malformed line")?;
    let duration = v["duration_ms"]
        .as_u64()
        .ok_or("traces unknown: malformed line")?;
    if ![
        "compress.run",
        "compress.hook",
        "compress.provider",
        "export.langfuse",
        "export.dashq",
    ]
    .contains(&op)
        || !["native", "filter", "batch"].contains(&step)
        || (!v["class"].is_null()
            && !v["class"].as_str().is_some_and(|s| {
                [
                    "auth",
                    "http",
                    "timeout",
                    "network",
                    "queue",
                    "no_reply",
                    "stopped",
                    "rejected",
                    "error",
                    "no_engine",
                ]
                .contains(&s)
            }))
    {
        return Err("traces unknown: malformed line".into());
    }
    Ok(json!({"time":at,"op":op,"step":step,"duration_ms":duration,"class":v["class"]}))
}
pub fn trace(data: &Path, v: Value) -> Result<(), String> {
    let v = trace_value(&v)?;
    let _lock = crate::executor::lock(data)?;
    let mut db = open(data)?;
    maintain(&mut db, crate::now(), v.to_string().len())?;
    let day = crate::date(v["time"].as_u64().unwrap());
    let bytes: i64 = db
        .query_row(
            "SELECT coalesce(sum(length(payload)),0) FROM events WHERE kind='trace' AND day=?1",
            [day],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if bytes + v.to_string().len() as i64 > crate::profile::TRACE_DAY_BYTES as i64 {
        return Err("trace day bound reached".into());
    }
    insert(&db, "trace", &crate::resource::new_id(), &v)?;
    checkpoint(&db)
}

pub fn traces(data: &Path) -> Result<Vec<Value>, String> {
    if !data.join("telemetry.sqlite3").exists() {
        return Ok(vec![]);
    }
    let _lock = crate::executor::lock(data)?;
    let db = open(data)?;
    events(&db, "trace")
}

pub fn sample(
    data: &Path,
    op: &str,
    at: u64,
    duration_ms: u64,
    class: Option<&str>,
) -> Result<(), String> {
    if !["export.langfuse", "export.dashq"].contains(&op) {
        return Err("unknown export operation".into());
    }
    let _lock = crate::executor::lock(data)?;
    let mut db = open(data)?;
    let v = json!({"time":at,"op":op,"duration_ms":duration_ms,"class":class});
    maintain(&mut db, crate::now(), v.to_string().len())?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    insert(&tx, "export", &crate::resource::new_id(), &v)?;
    tx.execute("DELETE FROM events WHERE kind='export' AND json_extract(payload,'$.op')=?1 AND seq NOT IN (SELECT seq FROM events WHERE kind='export' AND json_extract(payload,'$.op')=?1 ORDER BY time DESC,seq DESC LIMIT 256)",[op]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    checkpoint(&db)
}

pub fn samples(data: &Path) -> Result<serde_json::Map<String, Value>, String> {
    if !data.join("telemetry.sqlite3").exists() {
        return Ok(serde_json::Map::new());
    }
    let _lock = crate::executor::lock(data)?;
    let db = open(data)?;
    let mut out = serde_json::Map::new();
    for v in events(&db, "export")? {
        let op = v["op"]
            .as_str()
            .ok_or("export state unknown: malformed sample")?;
        let samples = out.entry(op).or_insert(json!([]));
        if let Some(a) = samples.as_array_mut() {
            a.push(json!([v["time"], v["duration_ms"], v["class"]]));
        }
    }
    for samples in out.values_mut().filter_map(Value::as_array_mut) {
        samples.sort_by_key(|s| s[0].as_u64().unwrap_or(0));
        samples.drain(..samples.len().saturating_sub(crate::profile::KEEP));
    }
    Ok(out)
}

/// Existing producer events outlive optional queue delivery; compression already has its canonical stats record.
pub fn event(data: &Path, span: &Value, at: u64) -> Result<(), String> {
    events_at(data, &[(span, at)])
}

/// `event` for many spans under one lock, maintenance pass, transaction and checkpoint (Claude model calls arrive by
/// the thousand). An oversized record is skipped and named; the others are kept.
pub fn events_at(data: &Path, spans: &[(&Value, u64)]) -> Result<(), String> {
    let mut rows = vec![];
    let mut skipped = None;
    for (span, at) in spans {
        let payload = span.to_string();
        if payload.len() > RECORD_MAX {
            skipped = Some("local telemetry record exceeds 64 KiB".to_string());
            continue;
        }
        let id = span["traceId"].as_str().ok_or("event missing trace ID")?.to_string();
        rows.push((id, *at, payload, span["name"] == "agentq.compress"));
    }
    if !rows.is_empty() {
        let _lock = crate::executor::lock(data)?;
        let mut db = open(data)?;
        maintain(&mut db, crate::now(), rows.iter().map(|r| r.2.len()).sum())?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for (id, at, payload, compress) in &rows {
            if *compress {
                let held: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM events WHERE kind='stats' AND id=?1)",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if held {
                    continue;
                }
            }
            let previous: Option<String> = tx
                .query_row(
                    "SELECT payload FROM events WHERE kind='event' AND id=?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if previous.as_deref() == Some(payload.as_str()) {
                continue;
            }
            tx.execute("INSERT INTO events(kind,id,time,day,payload) VALUES('event',?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET time=excluded.time,day=excluded.day,payload=excluded.payload",
                params![id,(*at).min(i64::MAX as u64) as i64,crate::date(*at),payload]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        checkpoint(&db)?;
    }
    skipped.map_or(Ok(()), Err)
}

pub fn export(data: &Path) -> Result<Value, String> {
    let _lock = crate::executor::lock(data)?;
    let mut db = open(data)?;
    let mut errors = serde_json::Map::new();
    maintain(&mut db, crate::now(), 0)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let journal = raw_meta(&tx, "journal")?
        .map(|s| serde_json::from_str::<Value>(&s).unwrap_or(json!("unreadable")));
    if !crate::local::failed(data).is_empty() {
        errors.insert("stats".into(), json!("stats write failed"));
    }
    Ok(
        json!({"schema":1,"collection_completeness":"unknown: calls that leave no evidence are undetectable","stats":events(&tx,"stats")?,"traces":events(&tx,"trace")?,"export_samples":events(&tx,"export")?,
        "events":events(&tx,"event")?,"coverage":journal,"errors":errors}),
    )
}

pub fn export_jsonl(data: &Path) -> Result<String, String> {
    let v = export(data)?;
    let mut lines=vec![json!({"schema":"agentq.telemetry/1","collection_completeness":v["collection_completeness"],"coverage":v["coverage"],"errors":v["errors"]}).to_string()];
    for (key, kind) in [
        ("stats", "stats"),
        ("traces", "trace"),
        ("export_samples", "export"),
        ("events", "event"),
    ] {
        for record in v[key].as_array().into_iter().flatten() {
            lines.push(json!({"kind":kind,"record":record}).to_string());
        }
    }
    Ok(lines.join("\n"))
}

/// Read-only store diagnostics; a readable database does not qualify compression or complete collection.
pub fn check_store(data: &Path) -> Result<u64, String> {
    let db = Connection::open_with_flags(
        data.join("telemetry.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| format!("store unavailable: {e}"))?;
    db.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    let app: i64 = db
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if app != APP || version != 1 {
        return Err("foreign or unsupported schema; file kept".into());
    }
    let integrity: String = db
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if integrity != "ok" {
        return Err("SQLite integrity check failed; file kept".into());
    }
    let count: i64 = db
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if !crate::local::failed(data).is_empty() {
        return Err("stats write failed".into());
    }
    Ok(count.max(0) as u64)
}

pub fn selftest() -> bool {
    let data =
        std::env::temp_dir().join(format!("agentq-sqlite-selftest-{}", crate::resource::new_id()));
    let result = (|| -> Result<(), String> {
        trace(
            &data,
            json!({"time":crate::now(),"op":"compress.run","step":"native","duration_ms":1,"class":null}),
        )?;
        sample(&data, "export.langfuse", crate::now(), 1, None)?;
        let exported = export(&data)?;
        if exported["traces"].as_array().is_none_or(|a| a.len() != 1)
            || exported["export_samples"]
                .as_array()
                .is_none_or(|a| a.len() != 1)
        {
            return Err("store readback mismatch".into());
        }
        check_store(&data)?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&data);
    result.is_ok()
}

/// Test rows and coverage that no current producer can write as such: old times, lost rows, malformed payloads.
#[cfg(test)]
pub fn seed(data: &Path, kind: &str, rows: &[Value], journal: Option<&Value>) {
    let db = open(data).unwrap();
    for r in rows {
        insert(&db, kind, &crate::resource::new_id(), r).unwrap();
    }
    if let Some(j) = journal {
        set(&db, "journal", j).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(at: u64) -> Value {
        json!({"time":at,"event_id":"invented-id"})
    }
    #[test]
    fn idempotent_retry_conflict_and_failed_transaction() {
        let d = std::env::temp_dir().join(format!("agentq-sqlite-{}", crate::resource::new_id()));
        let mut db = open(&d).unwrap();
        checkpoint(&db).unwrap();
        assert!(insert(&db, "stats", "invented-id", &record(1)).unwrap());
        assert!(!insert(&db, "stats", "invented-id", &record(1)).unwrap());
        assert!(
            insert(&db, "stats", "invented-id", &record(2))
                .unwrap_err()
                .contains("conflicting")
        );
        let tx = db.transaction().unwrap();
        insert(&tx, "stats", "second", &record(2)).unwrap();
        drop(tx);
        assert_eq!(events(&db, "stats").unwrap(), vec![record(1)]);
        drop(db);
        std::fs::remove_dir_all(d).unwrap();
    }
    fn dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("agentq-store-{}", crate::resource::new_id()))
    }
    fn valid(at: u64) -> Value {
        json!({"schema":1,"event_id":crate::resource::new_id().replace('-',""),"time":at,"agent":"hermes","adapter":"hook","tool":"terminal","runner":null,
            "engine":"none","outcome":"passed","reason":"short","bytes_in":10,"bytes_out":10,"est_tokens_in":3,"est_tokens_out":3,"duration_ms":1,
            "ref":null,"session":null,"turn":null,"tool_call":null})
    }
    #[test]
    fn acknowledged_record_and_coverage_survive_kill() {
        if let Some(path) = std::env::var_os("AGENTQ_SQLITE_CRASH_TEST") {
            let d = Path::new(&path);
            save_locked(d, &valid(crate::now()), false).unwrap();
            let db = open(d).unwrap();
            db.execute_batch("BEGIN IMMEDIATE").unwrap();
            let v = valid(crate::now());
            insert(&db, "stats", v["event_id"].as_str().unwrap(), &v).unwrap();
            std::fs::write(d.join("ack"), b"committed first record and journal").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(30));
            panic!("parent did not kill child");
        }
        let d = dir();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "local::tests::acknowledged_record_and_coverage_survive_kill",
                "--nocapture",
            ])
            .env("AGENTQ_SQLITE_CRASH_TEST", &d)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !d.join("ack").exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "writer exited before acknowledgement"
            );
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("writer did not acknowledge");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        let s = snapshot(&d).unwrap();
        assert_eq!(s["records"].as_array().unwrap().len(), 1);
        assert_eq!(s["journal"]["counts"], s["held"]);
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn corrupt_and_newer_database_are_preserved() {
        let d = dir();
        let db = open(&d).unwrap();
        db.pragma_update(None, "user_version", 2).unwrap();
        drop(db);
        assert!(snapshot(&d).unwrap_err().contains("unsupported"));
        let p = d.join("telemetry.sqlite3");
        std::fs::write(&p, b"invented foreign bytes").unwrap();
        assert!(snapshot(&d).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"invented foreign bytes");
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn pinned_reader_refuses_additional_writes() {
        let d = dir();
        save_locked(&d, &valid(crate::now()), false).unwrap();
        let reader = open(&d).unwrap();
        reader.execute_batch("BEGIN; SELECT * FROM events").unwrap();
        let writer = open(&d).unwrap();
        let v = valid(crate::now());
        insert(&writer, "stats", v["event_id"].as_str().unwrap(), &v).unwrap();
        assert!(
            sample(&d, "export.langfuse", crate::now(), 1, None)
                .unwrap_err()
                .contains("checkpoint busy")
        );
        reader.execute_batch("ROLLBACK").unwrap();
        drop(reader);
        drop(writer);
        assert_eq!(
            snapshot(&d).unwrap()["records"].as_array().unwrap().len(),
            2
        );
        assert!(samples(&d).unwrap().is_empty());
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn pressure_retention_and_oversized_refusal_keep_coverage_honest() {
        let d = dir();
        let now = crate::now();
        let mut last = Value::Null;
        for _ in 0..600 {
            let mut v = valid(now);
            v["session"] = json!("x".repeat(60 << 10));
            save_locked(&d, &v, false).unwrap();
            last = v;
            let bytes: u64 = std::fs::read_dir(&d)
                .unwrap()
                .map(|e| e.unwrap().metadata().unwrap().len())
                .sum();
            assert!(bytes < 64 << 20);
        }
        let s = snapshot(&d).unwrap();
        let held = s["records"].as_array().unwrap();
        assert!(!held.is_empty() && held.len() < 600);
        assert_eq!(held.last(), Some(&last));
        assert_eq!(s["journal"]["counts"], s["held"]);
        assert!(s["journal"]["since"].as_u64().unwrap() > now);
        let mut big = valid(now);
        big["session"] = json!("x".repeat(RECORD_MAX + 1));
        assert!(save_locked(&d, &big, false).is_err());
        assert_eq!(
            snapshot(&d).unwrap()["records"],
            s["records"],
            "invalid input must not prune existing history"
        );
        let mut db = open(&d).unwrap();
        maintain(&mut db, now + WINDOW + 1, 0).unwrap();
        assert!(events(&db, "stats").unwrap().is_empty());
        drop(db);
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn profile_history_is_local_bounded_and_does_not_merge_equal_samples() {
        let d = dir();
        let now = crate::now();
        let v =
            json!({"time":now,"op":"compress.run","step":"native","duration_ms":1,"class":null});
        for _ in 0..3 {
            trace(&d, v.clone()).unwrap();
        }
        assert_eq!(traces(&d).unwrap().len(), 3);
        let held = |d: &Path| samples(d).unwrap()["export.langfuse"].as_array().unwrap().len();
        for _ in 0..3 {
            sample(&d, "export.langfuse", now, 1, None).unwrap();
        }
        assert_eq!(held(&d), 3);
        for _ in 0..300 {
            sample(&d, "export.langfuse", now, 1, None).unwrap();
        }
        assert_eq!(held(&d), 256);
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn existing_events_survive_queue_delivery_and_resource_id_replaces_once() {
        let d = dir();
        let id = "1".repeat(32);
        for n in [1, 2] {
            crate::telemetry::emit(
                &d,
                "agentq.resource",
                "sample",
                json!({"agentq.event_id":id,"agentq.resource.test":n}),
            )
            .unwrap();
        }
        for (_, p, _) in crate::telemetry::entries(&crate::telemetry::queue(&d)) {
            std::fs::remove_file(p).unwrap();
        }
        let s = export(&d).unwrap();
        let events = s["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["traceId"], id);
        assert!(
            events[0]["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["key"] == "agentq.resource.test" && a["value"]["intValue"] == 2)
        );
        assert!(crate::telemetry::entries(&crate::telemetry::queue(&d)).is_empty());
        std::fs::remove_dir_all(d).unwrap();
    }
}

// Retained historical compression metadata remains readable; no optimizer runs here.
pub const FAILED: &str = "coverage.failed";

/// Failure markers held in `data`.
pub fn failed(data: &Path) -> Vec<std::path::PathBuf> {
    let es = std::fs::read_dir(data).into_iter().flatten().flatten();
    es.filter(|e| e.file_name().to_string_lossy().starts_with(FAILED)).map(|e| e.path()).collect()
}

#[cfg(test)]
pub(crate) fn coverage(journal: Option<&[u8]>, held: bool, lines: &serde_json::Map<String, Value>, at: u64, reset: u64) -> Value {
    let fresh = |since: u64, resets: Vec<u64>| json!({"schema":1,"since":since,"resets":resets,"gaps":[],"counts":lines});
    // A reset proves only the next second on: an earlier call in its own second may already be lost.
    match journal {
        None if !held => fresh(at, vec![]),
        None => fresh(reset + 1, vec![reset]),
        Some(b) => match serde_json::from_slice::<Value>(b) {
            Ok(v) if journal_valid(&v) => v,
            _ => fresh(reset + 1, vec![reset]),
        },
    }
}

/// The whole journal shape, as the producer writes it; anything else proves no coverage.
pub fn journal_valid(v: &Value) -> bool {
    v["schema"] == 1
        && v["since"].is_u64()
        && v["resets"]
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_u64))
        && v["gaps"].as_array().is_some_and(|a| {
            a.iter().all(|g| {
                g.as_array().is_some_and(|p| p.len() == 2)
                    && g[0].as_u64().zip(g[1].as_u64()).is_some_and(|(x, y)| x < y)
            })
        })
        && v["counts"]
            .as_object()
            .is_some_and(|m| m.iter().all(|(k, n)| stats_date(&format!("{k}.jsonl")).is_some() && n.is_u64()))
}

#[cfg(test)]
pub(crate) fn validate_record(v: &Value) -> Result<(), String> {
    v["agent"].as_str().ok_or("stats unknown: missing agent")?;
    v["engine"]
        .as_str()
        .filter(|e| ["none", "rtk", "headroom"].contains(e))
        .ok_or("stats unknown: invalid engine")?;
    v["outcome"]
        .as_str()
        .filter(|o| ["compressed", "passed", "failed"].contains(o))
        .ok_or("stats unknown: invalid outcome")?;
    if v["schema"] != 1 {
        return Err("stats unknown: unsupported schema".into());
    }
    if !v["event_id"]
        .as_str()
        .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        || ["time", "duration_ms"]
            .iter()
            .any(|k| v[k].as_u64().is_none())
        || ["bytes_in", "bytes_out", "est_tokens_in", "est_tokens_out"]
            .iter()
            .any(|k| {
                v.get(k).is_none()
                    || (v[k].as_u64().is_none()
                        && !(v["adapter"] == "run"
                            && v[k].is_null()
                            && v["outcome"] != "compressed"))
            })
        || !v["adapter"]
            .as_str()
            .is_some_and(|s| ["run", "hook", "provider"].contains(&s))
        || v["tool"].as_str().is_none()
        || !v["reason"]
            .as_str()
            .and_then(crate::accounting::outcome_of)
            .is_some_and(|o| v["outcome"] == o)
        || ["runner", "ref", "session", "turn", "tool_call"]
            .iter()
            .any(|k| v.get(k).is_none() || (!v[k].is_null() && !v[k].is_string()))
        || (v["adapter"] == "run" && v["exit_code"].as_i64().is_none())
    {
        return Err("stats unknown: malformed record shape".into());
    }
    Ok(())
}

pub(crate) fn stats_date(name: &str) -> Option<&str> {
    let d = name.strip_suffix(".jsonl")?;
    (d.len() == 10
        && d.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        }))
    .then_some(d)
}
