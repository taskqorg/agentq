// M6.12: caller labels and native-looking formats cannot authorize family/media discovery.
#[test]
fn caller_scope_and_explicit_hook_witness() {
    let (data, mut c) = setup("archive-discovery-authority");
    c.runtime = "codex".into();
    c.manifest["runtime"] = json!("codex");
    c.sources[0].format = "codex-app-server-selected/1".into();
    let selected = b"{\"method\":\"item/completed\",\"params\":{\"threadId\":\"s-1\",\"turnId\":\"t-1\",\"item\":{\"id\":\"own\",\"type\":\"agentMessage\",\"text\":\"selected only\"}}}\n";
    fs::write(&c.sources[0].path, selected).unwrap();
    let native = c.home.as_ref().unwrap().join("sessions/native.jsonl");
    fs::create_dir_all(native.parent().unwrap()).unwrap();
    fs::write(&native, b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"s-1\"}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"native only\"}}\n").unwrap();
    let image = c
        .home
        .as_ref()
        .unwrap()
        .join("generated_images/s-1/unreferenced.png");
    fs::create_dir_all(image.parent().unwrap()).unwrap();
    fs::write(&image, b"\x89PNG\r\n\x1a\nnative asset").unwrap();
    register(&data, c.clone()).unwrap();
    let mut sink = Sink::default();
    pass(&data, &mut sink).unwrap();
    assert_eq!(sink.requests.len(), 1);
    assert_eq!(sink.requests[0]["bytes"], agent::encode_base64(selected));
    assert_eq!(sink.requests[0]["format"], "codex-app-server-selected/1");
    // Even an explicit selected file is not an independently valid native witness.
    register_candidate(
        &data,
        c.clone(),
        Some(NativeCandidate::Hook(c.sources[0].path.clone())),
    )
    .unwrap();
    pass(&data, &mut sink).unwrap();
    assert_eq!(sink.requests.len(), 1);
    let explicit = c.root.join("hook-transcript.jsonl");
    fs::copy(&native, &explicit).unwrap();
    c.manifest["entry"] = json!("hook");
    c.sources[0].format = "native".into();
    c.sources[0].path = explicit.clone();
    register(&data, c.clone()).unwrap();
    pass(&data, &mut sink).unwrap();
    assert_eq!(
        sink.requests.len(),
        1,
        "public native format is caller-only"
    );
    register_candidate(
        &data,
        c.clone(),
        Some(NativeCandidate::Hook(explicit.clone())),
    )
    .unwrap();
    let saved: Registration = read(&files(&root(&data).join("captures")).unwrap()[0]).unwrap();
    let witness = saved.native.unwrap();
    assert!(witness.checkpoint["identity"].is_string());
    assert!(witness.checkpoint["offset"].as_u64().unwrap() > 0);
    pass(&data, &mut sink).unwrap();
    assert!(sink.requests.iter().any(|b| b["format"] == "codex-rollout"));
    assert!(sink.requests.iter().any(
        |b| b["media"] == true && b["source"] == crate::sha256_hex(&fs::read(&image).unwrap())
    ));
    let source_states: Vec<Checkpoint> = files(&root(&data).join("sources"))
        .unwrap()
        .iter()
        .map(|p| read(p).unwrap())
        .collect();
    assert!(
        source_states
            .iter()
            .any(|st| st.paths.contains(&explicit) && st.raw > 0)
    );
    assert_eq!(status(&data)["state"], "observed");
    let n = sink.requests.len();
    // Prefix/identity changes revoke new native discovery; no label fallback.
    fs::write(
        &explicit,
        b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"foreign\"}}\n",
    )
    .unwrap();
    fs::write(image.with_file_name("late.png"), b"\x89PNG\r\n\x1a\nlate").unwrap();
    pass(&data, &mut sink).unwrap();
    assert_eq!(sink.requests.len(), n);
    let _ = fs::remove_dir_all(data.parent().unwrap());
}

// M6.12: queued bytes keep their original context through re-registration and generation changes.
#[test]
fn queued_authority_cannot_be_adopted_and_legacy_fails_closed() {
    for change in ["class", "owner", "project"] {
        let (data, mut c) = setup(&format!("archive-original-{change}"));
        let grant_file = connection(&data).join("connection.json");
        let mut connection: Value = read(&grant_file).unwrap();
        connection["active"]["capabilities"] = json!([MANUAL]);
        write(&grant_file, &connection).unwrap();
        c.capture = "manual".into();
        register(&data, c.clone()).unwrap();
        let mut sink = Sink {
            offline: true,
            ..Sink::default()
        };
        pass(&data, &mut sink).unwrap();
        let checkpoint_path = files(&root(&data).join("sources")).unwrap()[0].clone();
        let original: Checkpoint = read(&checkpoint_path).unwrap();
        let original_capture = c.clone();
        let pending = original.pending[0].body.clone();
        match change {
            "class" => {
                connection["active"]["capabilities"] = json!([]);
                write(&grant_file, &connection).unwrap();
                c.capture = "launched".into();
            }
            "owner" => {
                fs::remove_file(files(&root(&data).join("captures")).unwrap()[0].clone()).unwrap();
                c.id = "different-owner".into();
            }
            "project" => {
                fs::write(
                    c.root.join(".agentq-archive.json"),
                    b"{\"exclude\":[\".\"]}",
                )
                .unwrap();
                c.root = data.parent().unwrap().join("project-b");
                fs::create_dir_all(&c.root).unwrap();
            }
            _ => unreachable!(),
        }
        register(&data, c).unwrap();
        sink.offline = false;
        pass(&data, &mut sink).unwrap();
        assert!(sink.requests.is_empty(), "{change} adopted old authority");
        let retained: Checkpoint = read(&checkpoint_path).unwrap();
        assert_eq!(retained.pending[0].body, pending);
        assert!(status(&data)["pending_bytes"].as_u64().unwrap() > 0);
        if change == "class" {
            connection["active"]["capabilities"] = json!([MANUAL]);
            write(&grant_file, &connection).unwrap();
            register(&data, original_capture.clone()).unwrap();
            fs::remove_file(&original_capture.sources[0].path).unwrap();
            pass(&data, &mut sink).unwrap();
            assert_eq!(sink.requests, vec![pending]);
            assert_eq!(status(&data)["pending_bytes"], 0);
        }
        let _ = fs::remove_dir_all(data.parent().unwrap());
    }
    let (data, c) = setup("archive-legacy-authority");
    register(&data, c.clone()).unwrap();
    let mut sink = Sink {
        offline: true,
        ..Sink::default()
    };
    pass(&data, &mut sink).unwrap();
    let path = files(&root(&data).join("sources")).unwrap()[0].clone();
    let mut legacy: Value = read(&path).unwrap();
    legacy.as_object_mut().unwrap().remove("authority");
    for q in legacy["pending"].as_array_mut().unwrap() {
        q.as_object_mut().unwrap().remove("authority");
    }
    write(&path, &legacy).unwrap();
    let bytes = fs::read(&path).unwrap();
    sink.offline = false;
    fs::remove_file(&c.sources[0].path).unwrap();
    pass(&data, &mut sink).unwrap();
    assert!(sink.requests.is_empty());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(status(&data)["pending_bytes"].as_u64().unwrap() > 0);
    let _ = fs::remove_dir_all(data.parent().unwrap());
}

// M6.12: both alias and original physical target exclusions survive loss, redirect and replacement.
#[cfg(unix)]
#[test]
fn canonical_source_exclusions_and_exact_source_loss_replay() {
    for change in ["loss", "redirect", "generation", "alias"] {
        let (data, c) = setup(&format!("archive-target-{change}"));
        let target = c.root.join("private/source.jsonl");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::rename(&c.sources[0].path, &target).unwrap();
        std::os::unix::fs::symlink(&target, &c.sources[0].path).unwrap();
        register(&data, c.clone()).unwrap();
        let mut sink = Sink {
            offline: true,
            ..Sink::default()
        };
        pass(&data, &mut sink).unwrap();
        let path = files(&root(&data).join("sources")).unwrap()[0].clone();
        let old: Checkpoint = read(&path).unwrap();
        assert!(old.paths.contains(&target.canonicalize().unwrap()));
        let pending = old.pending[0].body.clone();
        fs::write(
            c.root.join(".agentq-archive.json"),
            if change == "alias" {
                b"{\"exclude\":[\"turn.jsonl\"]}".as_slice()
            } else {
                b"{\"exclude\":[\"private/\"]}".as_slice()
            },
        )
        .unwrap();
        sink.offline = false;
        pass(&data, &mut sink).unwrap();
        assert!(sink.requests.is_empty());
        fs::remove_file(&c.sources[0].path).unwrap();
        if change == "redirect" || change == "generation" {
            let replacement = c.root.join("replacement.jsonl");
            fs::write(&replacement, b"{\"replacement\":true}\n").unwrap();
            std::os::unix::fs::symlink(&replacement, &c.sources[0].path).unwrap();
        }
        pass(&data, &mut sink).unwrap();
        assert!(!sink.requests.iter().any(|b| b == &pending));
        let retained: Checkpoint = read(&path).unwrap();
        assert_eq!(retained.pending[0].body, pending);
        fs::remove_file(c.root.join(".agentq-archive.json")).unwrap();
        pass(&data, &mut sink).unwrap();
        assert!(
            sink.requests.iter().any(|b| b == &pending),
            "original exact replay lost"
        );
        if change == "generation" {
            assert!(status(&data)["replaced"].as_u64().unwrap() > 0);
        }
        let _ = fs::remove_dir_all(data.parent().unwrap());
    }
}

// M6.12: the classifier and caller reader see the same authorized physical segment.
#[test]
fn bounded_unknown_session_and_condition_format_activation() {
    let (data, mut c) = setup("archive-bounded-unknown");
    c.runtime = "codex".into();
    c.manifest["runtime"] = json!("codex");
    c.manifest["session"] = Value::Null;
    c.sources[0].format = "codex-exec".into();
    let prefix = b"{\"type\":\"thread.started\",\"thread_id\":\"foreign-before\"}\n";
    let bounded = b"{\"type\":\"item.completed\",\"item\":{\"id\":\"own\",\"type\":\"agent_message\",\"text\":\"bounded\"}}\n";
    let later = b"{\"type\":\"thread.started\",\"thread_id\":\"foreign-after\"}\n";
    fs::write(
        &c.sources[0].path,
        [prefix.as_slice(), bounded, later].concat(),
    )
    .unwrap();
    c.sources[0].start = prefix.len() as u64;
    c.sources[0].end = Some((prefix.len() + bounded.len()) as u64);
    register(&data, c.clone()).unwrap();
    let mut sink = Sink::default();
    pass(&data, &mut sink).unwrap();
    assert_eq!(sink.requests.len(), 1);
    assert_eq!(sink.requests[0]["manifest"]["session"], "unknown");
    assert_eq!(sink.requests[0]["bytes"], agent::encode_base64(bounded));
    let first = sink.requests[0].clone();
    c.manifest["model"] = json!("new-model");
    register(&data, c.clone()).unwrap();
    pass(&data, &mut sink).unwrap();
    assert_eq!(
        sink.requests.len(),
        1,
        "same-size conditions must not activate"
    );
    fs::write(
        &c.sources[0].path,
        [prefix.as_slice(), bounded, bounded].concat(),
    )
    .unwrap();
    c.sources[0].end = Some((prefix.len() + 2 * bounded.len()) as u64);
    register(&data, c.clone()).unwrap();
    pass(&data, &mut sink).unwrap();
    assert_eq!(sink.requests.len(), 2);
    assert_eq!(sink.requests[1]["start"], first["end"]);
    assert_eq!(sink.requests[1]["manifest"]["model"], "new-model");
    c.sources[0].format = "raw".into();
    register(&data, c).unwrap();
    pass(&data, &mut sink).unwrap();
    assert_eq!(sink.requests.len(), 3);
    assert_ne!(sink.requests[2]["source"], first["source"]);
    assert!(
        sink.requests
            .iter()
            .all(|b| b["manifest"].as_object().unwrap().len() == 15)
    );
    let _ = fs::remove_dir_all(data.parent().unwrap());
}

// The writer rechecks the observed physical tuple; a per-turn source is never relabeled as unbounded.
#[test]
fn observed_native_handoff_preserves_segment_and_revalidates_delayed_proof() {
    for change in [
        "append",
        "identity",
        "prefix",
        "truncated",
        "session",
        "root",
        "home",
        "missing",
    ] {
        let (data, mut c) = setup(&format!("archive-observed-{change}"));
        c.runtime = "codex".into();
        c.manifest["runtime"] = json!("codex");
        c.manifest["session"] = json!("native-session");
        let path = c.root.join("external/sessions/native.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let header = format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":"native-session","cwd":c.root}})
        );
        let own = format!(
            "{}\n",
            json!({"type":"event_msg","payload":{"type":"agent_message","message":"owned segment"}})
        );
        fs::write(&path, format!("{header}{own}")).unwrap();
        let bytes = fs::read(&path).unwrap();
        let file = fs::File::open(&path).unwrap();
        let mut proof = NativeSource {
            runtime: c.runtime.clone(),
            root: c.root.clone(),
            home: c.home.clone(),
            session: "native-session".into(),
            path: path.clone(),
            start: header.len() as u64,
            identity: agent::archive_file_identity(&file).unwrap(),
            observed_end: bytes.len() as u64,
            prefix: crate::sha256_hex(&bytes),
        };
        c.sources[0] = Source {
            path: path.clone(),
            format: "codex-rollout".into(),
            start: proof.start,
            end: Some(proof.observed_end),
        };
        let image = c
            .root
            .join("external/generated_images/native-session/own.png");
        fs::create_dir_all(image.parent().unwrap()).unwrap();
        fs::write(&image, b"\x89PNG\r\n\x1a\nonly native").unwrap();
        match change {
            "append" => {
                let mut b = bytes.clone();
                b.extend_from_slice(own.as_bytes());
                fs::write(&path, b).unwrap();
            }
            "identity" => proof.identity = "wrong physical identity".into(),
            "prefix" => proof.prefix = crate::sha256_hex(b"different bytes"),
            "truncated" => proof.observed_end += 1,
            "session" => proof.session = "foreign".into(),
            "root" => proof.root = c.root.join("foreign"),
            "home" => proof.home = Some(c.root.join("foreign-home")),
            "missing" => {
                fs::remove_file(&path).unwrap();
            }
            _ => unreachable!(),
        }
        // Observe updates are pure when the handle has no absolute data root; writer validation is exercised below.
        let mut handle = Handle {
            data: PathBuf::new(),
            capture: c.clone(),
            native: None,
            native_run: None,
        };
        let end = c.sources[0].end;
        handle.observe_native(proof.clone(), end);
        assert_eq!(handle.capture.sources[0].start, c.sources[0].start);
        assert_eq!(handle.capture.sources[0].end, end);
        register_candidate(&data, c.clone(), Some(NativeCandidate::Observed(proof))).unwrap();
        let registration: Registration =
            read(&files(&root(&data).join("captures")).unwrap()[0]).unwrap();
        assert_eq!(
            registration.native.is_some(),
            change == "append",
            "{change}"
        );
        let mut sink = Sink::default();
        pass(&data, &mut sink).unwrap();
        if change == "append" {
            assert!(
                sink.requests
                    .iter()
                    .any(|b| b["bytes"] == agent::encode_base64(own.as_bytes())
                        && b["manifest"]["task"] == 283)
            );
            assert!(
                sink.requests
                    .iter()
                    .any(|b| b["media"] == true && b["manifest"]["task"].is_null())
            );
            assert!(
                sink.requests
                    .iter()
                    .all(|b| b["manifest"].as_object().unwrap().len() == 15)
            );
        } else {
            assert!(
                sink.requests.iter().all(|b| b["media"] != true),
                "{change} broadened caller scope"
            );
        }
        let _ = fs::remove_dir_all(data.parent().unwrap());
    }
}
