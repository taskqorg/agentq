//! Installation-owned SQLite telemetry; public entrypoints serialize through the existing lifecycle lock.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::io::Read;
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
fn meta(db: &Connection, key: &str) -> Result<Option<Value>, String> {
    raw_meta(db, key)?
        .map(|s| serde_json::from_str(&s).map_err(|e| format!("telemetry metadata: {e}")))
        .transpose()
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

fn maintain(db: &mut Connection, now: u64, bytes: usize, through: u64) -> Result<(), String> {
    let reserve = (bytes as i64).div_euclid(4096) * 2 + 34;
    if reserve >= PAGES {
        return Err("local telemetry import batch exceeds capacity; history kept".into());
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
            let mut q=tx.prepare("SELECT seq,kind,day,time FROM events WHERE time < ?1 OR (?2 AND time<=?3) ORDER BY time,seq LIMIT 256").map_err(|e|e.to_string())?;
            q.query_map(
                params![
                    now.saturating_sub(WINDOW).min(i64::MAX as u64) as i64,
                    pressure,
                    through.min(i64::MAX as u64) as i64
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
            .filter(crate::compression::journal_valid);
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

// Historical import must not evict newer observations merely to replay an older file.
fn skip_import(db: &mut Connection, at: u64) -> Result<(), String> {
    checkpoint(db)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    if let Some(mut j) = raw_meta(&tx, "journal")?
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(crate::compression::journal_valid)
    {
        j["since"] = json!(j["since"].as_u64().unwrap_or(0).max(at.saturating_add(1)));
        set(&tx, "journal", &j)?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    checkpoint(db)
}

fn manifest(dir: &Path, now: u64) -> Result<Value, String> {
    let mut entries = vec![];
    let mut bytes = 0u64;
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let n = e.file_name().to_string_lossy().into_owned();
        if n != crate::compression::COVERAGE
            && !crate::compression::stats_date(&n)
                .is_some_and(|d| d >= crate::date(now.saturating_sub(WINDOW)).as_str())
        {
            continue;
        }
        let m = e.metadata().map_err(|e| e.to_string())?;
        bytes = bytes.saturating_add(m.len());
        if bytes > 64 << 20 {
            return Err("legacy telemetry exceeds 64 MiB import bound; files kept".into());
        }
        let at = m
            .modified()
            .map_err(|e| e.to_string())?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
            .to_string();
        entries.push(json!([n, m.len(), at]));
    }
    entries.sort_by_key(Value::to_string);
    let value = json!({"oldest_day":crate::date(now.saturating_sub(WINDOW)),"files":entries});
    if value.to_string().len() > RECORD_MAX {
        return Err("legacy manifest exceeds bound; files kept".into());
    }
    Ok(value)
}
// Keep old files and checkpoint each bounded transaction; no content-based event deduplication.
fn migrate(db: &mut Connection, data: &Path, now: u64) -> Result<(), String> {
    if !data.join("stats").exists() {
        return Ok(());
    }
    let manifest = manifest(&data.join("stats"), now)?;
    if meta(db, "legacy_manifest")?.as_ref() == Some(&manifest) {
        return Ok(());
    }
    let legacy = crate::accounting::legacy(data, now);
    if legacy["unknown"]
        .as_str()
        .is_some_and(|s| ["stats file unreadable", "stats unreadable"].contains(&s))
    {
        return Err(legacy["unknown"].as_str().unwrap().into());
    }
    let records = legacy["records"]
        .as_array()
        .ok_or("invalid legacy source")?;
    let mut seen = std::collections::HashMap::new();
    for r in records {
        if !r.is_object() {
            return Err("stats unknown: malformed record".into());
        }
        crate::compression::validate_record(r)?;
        if r.to_string().len() > RECORD_MAX {
            return Err("legacy record exceeds 64 KiB; file kept".into());
        }
        let id = r["event_id"].as_str().unwrap();
        if seen.insert(id, r).is_some_and(|old| old != r) {
            return Err("conflicting legacy event ID; file kept".into());
        }
        exists(db, "stats", id, r)?;
    }
    let old_journal = std::fs::read(data.join("stats").join(crate::compression::COVERAGE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(crate::compression::journal_valid);
    if old_journal
        .as_ref()
        .is_some_and(|v| v.to_string().len() > RECORD_MAX)
    {
        return Err("legacy coverage exceeds bound; file kept".into());
    }
    let first = meta(db, "legacy_manifest")?.is_none() && meta(db, "importing_stats")?.is_none();
    checkpoint(db)?;
    {
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if first && raw_meta(&tx, "journal")?.is_none() {
            if let Some(old) = old_journal.as_ref() {
                let mut j = json!({"schema":1,"since":old["since"],"resets":old["resets"],"gaps":old["gaps"],"counts":counts(&tx)?});
                let mut held = serde_json::Map::new();
                for r in records {
                    let day = crate::date(r["time"].as_u64().unwrap());
                    let n = held.get(&day).and_then(Value::as_u64).unwrap_or(0);
                    held.insert(day, json!(n + 1));
                }
                for (day, n) in old["counts"].as_object().unwrap() {
                    let missing = n
                        .as_u64()
                        .unwrap_or(0)
                        .saturating_sub(held.get(day).and_then(Value::as_u64).unwrap_or(0));
                    if missing > 0 {
                        j["counts"][day] = json!(
                            j["counts"][day]
                                .as_u64()
                                .unwrap_or(0)
                                .checked_add(missing)
                                .ok_or("coverage count overflow; history kept")?
                        );
                    }
                }
                set(&tx, "journal", &j)?;
            }
        }
        set(&tx, "importing_stats", &manifest)?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    for file in manifest["files"].as_array().unwrap() {
        let name = file[0].as_str().unwrap();
        if crate::compression::stats_date(name).is_none() {
            continue;
        }
        let mut text = String::new();
        std::fs::File::open(data.join("stats").join(name))
            .map_err(|e| e.to_string())?
            .take((64 << 20) + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() > 64 << 20 {
            return Err("legacy telemetry exceeds import bound; file kept".into());
        }
        let key = format!("legacy_cursor:{name}");
        let cursor = meta(db, &key)?;
        let mut offset = 0;
        if let Some(c) = cursor.as_ref() {
            let n = c["bytes"].as_u64().ok_or("legacy cursor malformed")? as usize;
            let hash = c["sha256"].as_str().ok_or("legacy cursor malformed")?;
            if text.get(..n).is_some_and(|s| {
                crate::resource::hex(
                    ring::digest::digest(&ring::digest::SHA256, s.as_bytes()).as_ref(),
                ) == hash
            }) {
                offset = n;
            } else {
                // The old age pruner may remove only the expired prefix; an ambiguous rewrite stays unknown.
                let id = c["last_id"].as_str().ok_or("legacy cursor malformed")?;
                let mut at = 0;
                let mut found = vec![];
                for line in text.split_inclusive('\n') {
                    at += line.len();
                    if serde_json::from_str::<Value>(line)
                        .ok()
                        .is_some_and(|r| r["event_id"] == id)
                    {
                        found.push(at);
                    }
                }
                if found.len() != 1 {
                    return Err(
                        "legacy stats rewritten or lost; files kept; coverage unknown".into(),
                    );
                }
                offset = found[0];
            }
        }
        let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
        hash.update(&text.as_bytes()[..offset]);
        while offset < text.len() {
            let mut end = offset;
            let mut batch = vec![];
            for line in text[offset..].split_inclusive('\n') {
                if !batch.is_empty() && end + line.len() - offset > RECORD_MAX {
                    break;
                }
                let r: Value =
                    serde_json::from_str(line).map_err(|_| "stats unknown: malformed record")?;
                crate::compression::validate_record(&r)?;
                if line.len() > RECORD_MAX {
                    return Err("legacy record exceeds 64 KiB; file kept".into());
                }
                end += line.len();
                batch.push(r);
            }
            let latest = batch
                .iter()
                .filter_map(|r| r["time"].as_u64())
                .max()
                .unwrap_or(0);
            let skip = match maintain(db, now, end - offset, latest) {
                Ok(()) => false,
                Err(e) if e == "local telemetry full; write refused" => {
                    skip_import(db, latest)?;
                    true
                }
                Err(e) => return Err(e),
            };
            hash.update(&text.as_bytes()[offset..end]);
            let tx = db.transaction().map_err(|e| e.to_string())?;
            let mut journal = raw_meta(&tx, "journal")?
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .filter(crate::compression::journal_valid);
            if !skip {
                for r in &batch {
                    if insert(&tx, "stats", r["event_id"].as_str().unwrap(), r)? {
                        if let Some(j) = journal.as_mut() {
                            let day = crate::date(r["time"].as_u64().unwrap());
                            j["counts"][&day] = json!(
                                j["counts"][&day]
                                    .as_u64()
                                    .unwrap_or(0)
                                    .checked_add(1)
                                    .ok_or("coverage count overflow; history kept")?
                            );
                        }
                    }
                }
            }
            if let Some(j) = journal {
                set(&tx, "journal", &j)?;
            }
            set(
                &tx,
                &key,
                &json!({"bytes":end,"sha256":crate::resource::hex(hash.clone().finish().as_ref()),"last_id":batch.last().unwrap()["event_id"]}),
            )?;
            tx.commit().map_err(|e| e.to_string())?;
            offset = end;
        }
    }
    maintain(db, now, 0, u64::MAX)?;
    checkpoint(db)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    if let (Some(mut j), Some(old)) = (
        raw_meta(&tx, "journal")?
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .filter(crate::compression::journal_valid),
        old_journal.as_ref(),
    ) {
        j["since"] = json!(
            j["since"]
                .as_u64()
                .unwrap_or(now + 1)
                .max(old["since"].as_u64().unwrap_or(now + 1))
        );
        for k in ["resets", "gaps"] {
            for v in old[k].as_array().unwrap() {
                if !j[k].as_array().unwrap().contains(v) {
                    j[k].as_array_mut().unwrap().push(v.clone());
                }
            }
        }
        set(&tx, "journal", &j)?;
    }
    let unknown = legacy["unknown"]
        .as_str()
        .filter(|s| *s != crate::accounting::UNPROVEN && *s != "stats write failed")
        .filter(|_| {
            !records.is_empty()
                || old_journal.as_ref().is_some_and(|j| {
                    j["counts"].as_object().unwrap().iter().any(|(day, n)| {
                        day.as_str() >= crate::date(now.saturating_sub(WINDOW)).as_str()
                            && n.as_u64().unwrap_or(0) > 0
                    })
                })
        })
        .map(Value::from)
        .unwrap_or(Value::Null);
    set(&tx, "legacy_unknown", &unknown)?;
    set(&tx, "legacy_manifest", &manifest)?;
    set(&tx, "legacy_error", &Value::Null)?;
    set(&tx, "importing_stats", &Value::Null)?;
    tx.execute("DELETE FROM meta WHERE key GLOB 'legacy_cursor:????-??-??.jsonl' AND substr(key,15,10) < ?1",[crate::date(now.saturating_sub(WINDOW))]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    checkpoint(db)
}

/// Called with the lifecycle lock held. A record and its coverage commit in one transaction.
pub fn save_locked(data: &Path, r: &Value, failed: bool) -> Result<(), String> {
    crate::compression::validate_record(r)?;
    if r.to_string().len() > RECORD_MAX {
        return Err("local telemetry record exceeds 64 KiB".into());
    }
    let mut db = open(data)?;
    if let Err(e) = migrate(&mut db, data, crate::now()) {
        checkpoint(&db)?;
        set(&db, "legacy_error", &json!(e))?;
    }
    maintain(&mut db, crate::now(), r.to_string().len(), u64::MAX)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let at = r["time"].as_u64().ok_or("missing time")?;
    let held = counts(&tx)?;
    let journal = raw_meta(&tx, "journal")?.map(String::into_bytes);
    let mut j = crate::compression::coverage(
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

pub fn snapshot(data: &Path) -> Result<Value, String> {
    if !data.join("telemetry.sqlite3").exists() && !data.join("stats").exists() {
        return Err("stats unknown: no stats file".into());
    }
    let _lock = crate::executor::lock(data)?;
    let mut db = open(data)?;
    let migrated = migrate(&mut db, data, crate::now());
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let error = migrated
        .err()
        .map(Value::from)
        .or(meta(&tx, "legacy_error")?)
        .unwrap_or(Value::Null);
    let journal = raw_meta(&tx, "journal")?
        .map(|s| serde_json::from_str::<Value>(&s).unwrap_or(json!("unreadable")));
    Ok(
        json!({"records":events(&tx,"stats")?,"journal":journal,"unknown":meta(&tx,"legacy_unknown")?,"error":error,"held":counts(&tx)?}),
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
fn migrate_traces(db: &mut Connection, data: &Path, now: u64) -> Result<(), String> {
    let dir = data.join("traces");
    if !dir.exists() {
        return Ok(());
    }
    let manifest = manifest(&dir, now).map_err(|e| format!("traces unknown: {e}"))?;
    if meta(db, "trace_manifest")?.as_ref() == Some(&manifest) {
        return Ok(());
    }
    for file in manifest["files"].as_array().unwrap() {
        let name = file[0].as_str().unwrap();
        if crate::compression::stats_date(name).is_none() {
            continue;
        }
        let mut text = String::new();
        std::fs::File::open(dir.join(name))
            .map_err(|e| format!("traces unknown: {e}"))?
            .take(crate::profile::TRACE_DAY_BYTES + RECORD_MAX as u64 + 1)
            .read_to_string(&mut text)
            .map_err(|e| format!("traces unknown: {e}"))?;
        if text.len() > crate::profile::TRACE_DAY_BYTES as usize + RECORD_MAX {
            return Err("traces unknown: legacy day exceeds bound; file kept".into());
        }
        let key = format!("trace_cursor:{name}");
        let cursor = meta(db, &key)?;
        let start = cursor
            .as_ref()
            .and_then(|c| c["bytes"].as_u64())
            .unwrap_or(0) as usize;
        if let Some(c) = cursor {
            if !text.get(..start).is_some_and(|s| {
                crate::resource::hex(
                    ring::digest::digest(&ring::digest::SHA256, s.as_bytes()).as_ref(),
                ) == c["sha256"]
            }) {
                return Err("traces unknown: legacy file rewritten; file kept".into());
            }
        }
        let mut rows = vec![];
        let mut offset = 0;
        for (index, line) in text.split_inclusive('\n').enumerate() {
            let v: Value =
                serde_json::from_str(line).map_err(|_| "traces unknown: malformed line")?;
            let v = trace_value(&v)?;
            let id = format!("{name}:{index}");
            exists(db, "trace", &id, &v)?;
            if offset >= start {
                rows.push((id, v));
            }
            offset += line.len();
        }
        let latest = rows
            .iter()
            .filter_map(|(_, v)| v["time"].as_u64())
            .max()
            .unwrap_or(0);
        let skip = match maintain(
            db,
            now,
            rows.iter().map(|(_, r)| r.to_string().len()).sum(),
            latest,
        ) {
            Ok(()) => false,
            Err(e) if e == "local telemetry full; write refused" => true,
            Err(e) => return Err(e),
        };
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if !skip {
            for (id, v) in rows {
                insert(&tx, "trace", &id, &v)?;
            }
        }
        set(
            &tx,
            &key,
            &json!({"bytes":text.len(),"sha256":crate::resource::hex(ring::digest::digest(&ring::digest::SHA256,text.as_bytes()).as_ref())}),
        )?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    checkpoint(db)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    set(&tx, "trace_manifest", &manifest)?;
    tx.execute("DELETE FROM meta WHERE key GLOB 'trace_cursor:????-??-??.jsonl' AND substr(key,14,10) < ?1",[crate::date(now.saturating_sub(WINDOW))]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    checkpoint(db)
}

pub fn trace(data: &Path, v: Value) -> Result<(), String> {
    let v = trace_value(&v)?;
    let _lock = crate::executor::lock(data)?;
    let mut db = open(data)?;
    let imported = migrate_traces(&mut db, data, crate::now());
    maintain(&mut db, crate::now(), v.to_string().len(), u64::MAX)?;
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
    insert(&db, "trace", &crate::mcp::new_id(), &v)?;
    checkpoint(&db)?;
    imported
}

pub fn traces(data: &Path, now: u64) -> Result<Vec<Value>, String> {
    if !data.join("telemetry.sqlite3").exists() && !data.join("traces").exists() {
        return Ok(vec![]);
    }
    let _lock = crate::executor::lock(data)?;
    let mut db = open(data)?;
    migrate_traces(&mut db, data, now)?;
    events(&db, "trace")
}

fn legacy_exports(data: &Path) -> Result<Value, String> {
    use std::io::Read;
    let mut text = String::new();
    match std::fs::File::open(data.join("queue/state.json")) {
        Ok(f) => {
            f.take((1 << 20) + 1)
                .read_to_string(&mut text)
                .map_err(|e| format!("export state unknown: {e}"))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(e) => return Err(format!("export state unknown: {e}")),
    }
    if text.len() > 1 << 20 {
        return Err("export state unknown: legacy state exceeds 1 MiB; file kept".into());
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|_| "export state unknown: unreadable state.json")?;
    let map = v
        .as_object()
        .ok_or("export state unknown: state.json is not a JSON object")?;
    let profile = match map.get("profile") {
        None => return Ok(json!({})),
        Some(Value::Object(p)) => p,
        _ => return Err("export state unknown: malformed profile".into()),
    };
    let mut out = json!({});
    for op in ["export.langfuse", "export.dashq"] {
        let Some(samples) = profile.get(op) else {
            continue;
        };
        let valid = samples.as_array().is_some_and(|a| {
            a.len() <= crate::profile::KEEP
                && a.iter().all(|s| {
                    s.as_array().is_some_and(|a| a.len() == 3)
                        && s[0].is_u64()
                        && s[1].is_u64()
                        && (s[2].is_null()
                            || s[2].as_str().is_some_and(|c| {
                                [
                                    "auth", "http", "timeout", "network", "queue", "no_reply",
                                    "stopped", "rejected",
                                ]
                                .contains(&c)
                            }))
                })
        });
        out[op] = if valid {
            samples.clone()
        } else {
            json!("malformed sample")
        };
    }
    Ok(out)
}
fn sync_exports(db: &Connection, data: &Path) -> Result<Value, String> {
    let legacy = legacy_exports(data)?;
    if meta(db, "legacy_exports")?.as_ref() != Some(&legacy) {
        checkpoint(db)?;
        set(db, "legacy_exports", &legacy)?;
        checkpoint(db)?;
    }
    Ok(legacy)
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
    let imported = sync_exports(&db, data);
    let v = json!({"time":at,"op":op,"duration_ms":duration_ms,"class":class});
    maintain(&mut db, crate::now(), v.to_string().len(), u64::MAX)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    insert(&tx, "export", &crate::mcp::new_id(), &v)?;
    tx.execute("DELETE FROM events WHERE kind='export' AND json_extract(payload,'$.op')=?1 AND seq NOT IN (SELECT seq FROM events WHERE kind='export' AND json_extract(payload,'$.op')=?1 ORDER BY time DESC,seq DESC LIMIT 256)",[op]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    checkpoint(&db)?;
    imported.map(|_| ())
}

pub fn samples(data: &Path) -> Result<serde_json::Map<String, Value>, String> {
    if !data.join("telemetry.sqlite3").exists() && !data.join("queue/state.json").exists() {
        return Ok(serde_json::Map::new());
    }
    let _lock = crate::executor::lock(data)?;
    let db = open(data)?;
    let mut out = sync_exports(&db, data)?
        .as_object()
        .cloned()
        .ok_or("export state unknown: malformed profile")?;
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
        maintain(&mut db, crate::now(), rows.iter().map(|r| r.2.len()).sum(), u64::MAX)?;
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
    if let Err(e) = migrate(&mut db, data, crate::now()) {
        errors.insert("stats".into(), json!(e));
    }
    if let Err(e) = migrate_traces(&mut db, data, crate::now()) {
        errors.insert("trace".into(), json!(e));
    }
    if let Err(e) = sync_exports(&db, data) {
        errors.insert("export".into(), json!(e));
    }
    maintain(&mut db, crate::now(), 0, u64::MAX)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let journal = raw_meta(&tx, "journal")?
        .map(|s| serde_json::from_str::<Value>(&s).unwrap_or(json!("unreadable")));
    if let Some(e) = meta(&tx, "legacy_error")?.filter(Value::is_string) {
        errors.insert("stats".into(), e);
    }
    if !crate::compression::failed(data).is_empty() {
        errors.insert("stats".into(), json!("stats write failed"));
    }
    Ok(
        json!({"schema":1,"collection_completeness":"unknown: calls that leave no evidence are undetectable","stats":events(&tx,"stats")?,"traces":events(&tx,"trace")?,"export_samples":events(&tx,"export")?,
        "legacy_export_samples":meta(&tx,"legacy_exports")?,"events":events(&tx,"event")?,"coverage":journal,"coverage_unknown":meta(&tx,"legacy_unknown")?,"errors":errors}),
    )
}

pub fn export_jsonl(data: &Path) -> Result<String, String> {
    let v = export(data)?;
    let mut lines=vec![json!({"schema":"agentq.telemetry/1","collection_completeness":v["collection_completeness"],"coverage":v["coverage"],"coverage_unknown":v["coverage_unknown"],"errors":v["errors"]}).to_string()];
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
    for (op, samples) in v["legacy_export_samples"].as_object().into_iter().flatten() {
        lines.push(json!({"kind":"legacy_export","operation":op,"samples":samples}).to_string());
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
    if meta(&db, "importing_stats")?.is_some_and(|v| !v.is_null()) {
        return Err("legacy import incomplete; retained rows kept".into());
    }
    let count: i64 = db
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if !crate::compression::failed(data).is_empty() {
        return Err("stats write failed".into());
    }
    Ok(count.max(0) as u64)
}

pub fn selftest() -> bool {
    let data =
        std::env::temp_dir().join(format!("agentq-sqlite-selftest-{}", crate::mcp::new_id()));
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

#[cfg(test)]
mod tests {
    use super::*;
    fn record(at: u64) -> Value {
        json!({"time":at,"event_id":"invented-id"})
    }
    #[test]
    fn idempotent_retry_conflict_and_failed_transaction() {
        let d = std::env::temp_dir().join(format!("agentq-sqlite-{}", crate::mcp::new_id()));
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
        std::env::temp_dir().join(format!("agentq-store-{}", crate::mcp::new_id()))
    }
    fn valid(at: u64) -> Value {
        json!({"schema":1,"event_id":crate::mcp::new_id().replace('-',""),"time":at,"agent":"hermes","adapter":"hook","tool":"terminal","runner":null,
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
        maintain(&mut db, now + WINDOW + 1, 0, u64::MAX).unwrap();
        assert!(events(&db, "stats").unwrap().is_empty());
        drop(db);
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn legacy_retry_equal_observations_and_mixed_writers() {
        let d = dir();
        std::fs::create_dir_all(d.join("stats")).unwrap();
        let now = crate::now();
        let day = crate::date(now);
        let first = valid(now);
        let path = d.join("stats").join(format!("{day}.jsonl"));
        std::fs::write(&path, format!("{first}\n")).unwrap();
        std::fs::write(
            d.join("stats/coverage.json"),
            json!({"schema":1,"since":now,"resets":[],"gaps":[],"counts":{&day:1}}).to_string(),
        )
        .unwrap();
        assert_eq!(
            snapshot(&d).unwrap()["records"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            snapshot(&d).unwrap()["records"].as_array().unwrap().len(),
            1
        );
        save_locked(&d, &valid(now), false).unwrap();
        let third = valid(now);
        std::fs::write(&path, format!("{first}\n{third}\n")).unwrap();
        assert_eq!(
            snapshot(&d).unwrap()["records"].as_array().unwrap().len(),
            3
        );
        let mut conflict = third.clone();
        conflict["bytes_in"] = json!(11);
        std::fs::write(&path, format!("{first}\n{conflict}\n")).unwrap();
        let s = snapshot(&d).unwrap();
        assert!(s["error"].as_str().unwrap().contains("conflicting"));
        assert_eq!(s["records"].as_array().unwrap().len(), 3);
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn profile_history_is_local_bounded_and_does_not_merge_equal_samples() {
        let d = dir();
        std::fs::create_dir_all(d.join("queue")).unwrap();
        std::fs::create_dir_all(d.join("traces")).unwrap();
        let now = crate::now();
        let v =
            json!({"time":now,"op":"compress.run","step":"native","duration_ms":1,"class":null});
        let trace_path = d.join("traces").join(format!("{}.jsonl", crate::date(now)));
        std::fs::write(&trace_path, format!("{v}\n{v}\n")).unwrap();
        trace(&d, v.clone()).unwrap();
        assert_eq!(traces(&d, now).unwrap().len(), 3);
        assert_eq!(traces(&d, now).unwrap().len(), 3);
        std::fs::write(&trace_path, format!("{v}\n{v}\n{v}\n")).unwrap();
        assert_eq!(traces(&d, now).unwrap().len(), 4);
        std::fs::write(
            d.join("queue/state.json"),
            json!({"profile":{"export.langfuse":[[now-1,1,null],[now-1,1,null]]}}).to_string(),
        )
        .unwrap();
        sample(&d, "export.langfuse", now, 1, None).unwrap();
        assert_eq!(
            samples(&d).unwrap()["export.langfuse"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            samples(&d).unwrap()["export.langfuse"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        for _ in 0..300 {
            sample(&d, "export.langfuse", now, 1, None).unwrap();
        }
        assert_eq!(
            samples(&d).unwrap()["export.langfuse"]
                .as_array()
                .unwrap()
                .len(),
            256
        );
        let state: Value =
            serde_json::from_str(&std::fs::read_to_string(d.join("queue/state.json")).unwrap())
                .unwrap();
        assert_eq!(
            state["profile"]["export.langfuse"]
                .as_array()
                .unwrap()
                .len(),
            2,
            "new producers never dual-write the legacy history"
        );
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

    #[test]
    fn large_same_second_import_resumes_by_position_not_payload_or_time() {
        let d = dir();
        std::fs::create_dir_all(d.join("stats")).unwrap();
        let now = crate::now();
        let day = crate::date(now);
        let mut text = String::new();
        let mut ids = vec![];
        for _ in 0..600 {
            let mut v = valid(now);
            v["session"] = json!("x".repeat(60 << 10));
            ids.push(v["event_id"].clone());
            text.push_str(&format!("{v}\n"));
        }
        let p = d.join("stats").join(format!("{day}.jsonl"));
        std::fs::write(&p, &text).unwrap();
        std::fs::write(
            d.join("stats/coverage.json"),
            json!({"schema":1,"since":now,"resets":[],"gaps":[],"counts":{&day:600}}).to_string(),
        )
        .unwrap();
        let s = snapshot(&d).unwrap();
        let rows = s["records"].as_array().unwrap();
        assert!(rows.len() < 600 && !rows.is_empty());
        assert_eq!(
            rows.last().unwrap()["event_id"],
            ids[599],
            "same-second later events must survive pressure pruning"
        );
        assert!(!rows.iter().any(|r| r["event_id"] == ids[0]));
        assert_eq!(s["journal"]["counts"], s["held"]);
        let late = valid(now);
        text.push_str(&format!("{late}\n"));
        std::fs::write(&p, &text).unwrap();
        let again = snapshot(&d).unwrap();
        let rows = again["records"].as_array().unwrap();
        assert!(rows.iter().any(|r| r["event_id"] == late["event_id"]));
        assert!(
            !rows.iter().any(|r| r["event_id"] == ids[0]),
            "a changed legacy file must not resurrect its already-consumed prefix"
        );
        assert_eq!(again["journal"]["counts"], again["held"]);
        std::fs::remove_dir_all(d).unwrap();
    }
}
