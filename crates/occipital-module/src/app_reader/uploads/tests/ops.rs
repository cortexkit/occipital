//! Every operation and every refusal code, through the frame loop.

use super::*;

const DAY: i64 = 24 * 60 * 60 * 1000;

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[tokio::test]
async fn an_upload_lands_in_the_project_and_is_answered_with_its_path() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(2_600);
    let begun = wire
        .call(PHONE_ROUTE, BEGIN, begin_params("clip.mov", &file))
        .await
        .ok();
    assert_eq!(begun["chunk_bytes"], 1_048_576);
    assert_eq!(begun["expires_at"], START + 24 * 60 * 60 * 1000);
    let id = begun["upload_id"].as_str().unwrap().to_owned();
    assert_eq!(id.len(), 32);
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 0})
    );
    for (at, piece) in [
        (0, &file[..1024]),
        (1024, &file[1024..2048]),
        (2048, &file[2048..]),
    ] {
        let reply = wire.chunk(&id, at, piece).await.ok();
        assert_eq!(reply, json!({"committed": at + piece.len() as u64}));
    }
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 2_600})
    );
    let finished = wire.id_call(FINISH, &id).await.ok();
    let path = f.date_dir().join("clip.mov");
    assert_eq!(
        finished,
        json!({"path": path.to_string_lossy(), "size": 2_600})
    );
    assert_eq!(std::fs::read(&path).unwrap(), file);
    assert_eq!(
        std::fs::read(f.project.join(".cortexkit/uploads/.gitignore")).unwrap(),
        b"*\n"
    );
    // Only the finished file is left in the date folder, and its blob is gone.
    assert_eq!(listing(&f.date_dir()), ["clip.mov"]);
    assert!(!f.blob(&id).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600, "never executable, never shared");
        for dir in [".cortexkit", ".cortexkit/uploads"] {
            assert_eq!(mode(&f.project.join(dir)), 0o700);
        }
        assert_eq!(mode(&f.date_dir()), 0o700);
    }
    // A finished upload still answers status, and abort leaves its file alone.
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 2_600})
    );
    assert_eq!(
        wire.id_call(ABORT, &id).await.ok(),
        json!({"aborted": false})
    );
    assert!(path.exists());
}

#[tokio::test]
async fn an_existing_gitignore_is_left_as_it_is() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.project.join(".cortexkit/uploads")).unwrap();
    std::fs::write(f.project.join(".cortexkit/uploads/.gitignore"), b"mine\n").unwrap();
    let mut wire = f.wire();
    wire.bind_all().await;
    wire.upload("a.bin", &content(10)).await;
    assert_eq!(
        std::fs::read(f.project.join(".cortexkit/uploads/.gitignore")).unwrap(),
        b"mine\n"
    );
}

#[tokio::test]
async fn abort_discards_the_blob_and_the_row() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(2_000);
    let id = wire.begin("a.bin", &file).await;
    wire.chunk(&id, 0, &file[..1000]).await.ok();
    assert!(f.blob(&id).exists());
    assert_eq!(
        wire.id_call(ABORT, &id).await.ok(),
        json!({"aborted": true})
    );
    assert!(!f.blob(&id).exists());
    assert_eq!(
        wire.id_call(STATUS, &id).await.code(),
        codes::UPLOAD_UNKNOWN
    );
    assert!(!f.project.join(".cortexkit").exists());
}

#[tokio::test]
async fn overlapping_pieces_must_match_the_bytes_already_held() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(3_000);
    let id = wire.begin("a.bin", &file).await;
    wire.chunk(&id, 0, &file[..1000]).await.ok();
    // The same piece again (its reply was lost) is accepted and changes nothing.
    assert_eq!(
        wire.chunk(&id, 0, &file[..1000]).await.ok(),
        json!({"committed": 1000})
    );
    // A piece straddling the prefix is compared where it overlaps, then extends it.
    assert_eq!(
        wire.chunk(&id, 500, &file[500..1500]).await.ok(),
        json!({"committed": 1500})
    );
    // Different bytes inside the prefix are refused and nothing moves.
    let mut wrong = file[1000..1500].to_vec();
    wrong[499] ^= 0xff;
    let refused = wire.chunk(&id, 1000, &wrong).await;
    assert_eq!(refused.code(), codes::CHUNK_MISMATCH);
    assert_eq!(refused.body()["detail"], json!({"committed": 1500}));
    // A piece straddling the prefix whose overlapping head differs is refused
    // whole: none of its new tail is taken either.
    let mut straddle = file[1400..2000].to_vec();
    straddle[0] ^= 0xff;
    assert_eq!(
        wire.chunk(&id, 1400, &straddle).await.code(),
        codes::CHUNK_MISMATCH
    );
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 1500})
    );
    assert_eq!(std::fs::read(f.blob(&id)).unwrap(), file[..1500]);
}

#[tokio::test]
async fn a_piece_past_the_prefix_is_refused_and_never_stored() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(3_000);
    let id = wire.begin("a.bin", &file).await;
    wire.chunk(&id, 0, &file[..1000]).await.ok();
    let refused = wire.chunk(&id, 2000, &file[2000..]).await;
    assert_eq!(refused.code(), codes::OFFSET_GAP);
    assert_eq!(refused.body()["detail"], json!({"committed": 1000}));
    assert_eq!(std::fs::read(f.blob(&id)).unwrap(), file[..1000]);
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 1000})
    );
}

#[tokio::test]
async fn sizes_are_capped_at_begin_and_pieces_cannot_pass_the_declared_size() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let mut params = begin_params("big.mov", b"");
    params["size"] = json!(MAX_FILE_BYTES + 1);
    let refused = wire.call(PHONE_ROUTE, BEGIN, params.clone()).await;
    assert_eq!(refused.code(), codes::UPLOAD_TOO_LARGE);
    params["size"] = json!(MAX_FILE_BYTES);
    wire.call(PHONE_ROUTE, BEGIN, params).await.ok();
    let file = content(100);
    let id = wire.begin("a.bin", &file).await;
    let mut longer = file.clone();
    longer.push(0);
    assert_eq!(
        wire.chunk(&id, 0, &longer).await.code(),
        codes::UPLOAD_TOO_LARGE
    );
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 0})
    );
}

#[tokio::test]
async fn begin_keeps_twenty_gib_free_on_both_volumes() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(1000);
    let floor = DISK_FLOOR_BYTES + 1000;
    for (data, project, volume) in [(floor - 1, floor, "data"), (floor, floor - 1, "project")] {
        f.free_data.store(data, Ordering::SeqCst);
        f.free_project.store(project, Ordering::SeqCst);
        let refused = wire
            .call(PHONE_ROUTE, BEGIN, begin_params("a.bin", &file))
            .await;
        assert_eq!(refused.code(), codes::DISK_FLOOR);
        assert_eq!(refused.body()["detail"]["volume"], volume);
    }
    f.free_data.store(floor, Ordering::SeqCst);
    f.free_project.store(floor, Ordering::SeqCst);
    let id = wire.begin("a.bin", &file).await;
    wire.chunk(&id, 0, &file).await.ok();
    // The project's volume is checked again at finish.
    f.free_project.store(floor - 1, Ordering::SeqCst);
    assert_eq!(wire.id_call(FINISH, &id).await.code(), codes::DISK_FLOOR);
    f.free_project.store(floor, Ordering::SeqCst);
    wire.id_call(FINISH, &id).await.ok();
}

#[tokio::test]
async fn unknown_and_malformed_ids_are_refused_by_name() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    for id in ["00000000000000000000000000000000", "../../store", ""] {
        for method in [STATUS, FINISH, ABORT] {
            assert_eq!(wire.id_call(method, id).await.code(), codes::UPLOAD_UNKNOWN);
        }
        assert_eq!(wire.chunk(id, 0, b"x").await.code(), codes::UPLOAD_UNKNOWN);
    }
}

#[tokio::test]
async fn sessions_resolve_through_entorhinal_and_a_held_one_is_refused() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let mut params = begin_params("a.bin", b"abc");
    params["session"] = json!(HELD);
    assert_eq!(
        wire.call(PHONE_ROUTE, BEGIN, params.clone()).await.code(),
        codes::SESSION_ON_OTHER_MACHINE
    );
    params["session"] = json!("nobody");
    assert_eq!(
        wire.call(PHONE_ROUTE, BEGIN, params).await.code(),
        "session_unknown"
    );
}

#[tokio::test]
async fn only_the_opening_device_may_touch_an_upload() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(2_000);
    let id = wire.begin("a.bin", &file).await;
    wire.chunk(&id, 0, &file[..1000]).await.ok();
    for channel in [OTHER_ROUTE, OPERATOR_ROUTE] {
        for method in [STATUS, FINISH, ABORT] {
            let reply = wire.call(channel, method, json!({"upload_id": id})).await;
            assert_eq!(
                reply.code(),
                codes::UPLOAD_NOT_YOURS,
                "{method} on {channel}"
            );
        }
        let (_, reply) = wire.chunk_on(channel, &id, 1000, &file[1000..]).await;
        assert_eq!(reply.code(), codes::UPLOAD_NOT_YOURS);
    }
    // Nothing the other callers sent reached the upload.
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 1000})
    );
    assert_eq!(std::fs::read(f.blob(&id)).unwrap(), file[..1000]);
    // A route without a device fingerprint can't open one either.
    let refused = wire
        .call(OPERATOR_ROUTE, BEGIN, begin_params("b.bin", &file))
        .await;
    assert_eq!(refused.code(), "carrier_op_not_permitted");
}

#[tokio::test]
async fn admission_is_the_app_readers_and_comes_before_the_params() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    for method in METHODS {
        // Params that would be invalid: admission must refuse first.
        let reply = wire
            .call(FOREIGN_ROUTE, method, json!("not an object"))
            .await;
        assert_eq!(reply.code(), "carrier_op_not_permitted", "{method}");
    }
}

#[tokio::test]
async fn an_upload_expires_after_a_day_and_its_record_after_thirty() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(2_000);
    let id = wire.begin("a.bin", &file).await;
    wire.chunk(&id, 0, &file[..1000]).await.ok();
    f.advance(DAY);
    assert_eq!(
        wire.chunk(&id, 1000, &file[1000..]).await.code(),
        codes::UPLOAD_EXPIRED
    );
    assert_eq!(
        wire.id_call(STATUS, &id).await.code(),
        codes::UPLOAD_EXPIRED
    );
    assert_eq!(
        wire.id_call(FINISH, &id).await.code(),
        codes::UPLOAD_EXPIRED
    );
    let swept = f.uploads.cleanup_pass().unwrap();
    assert_eq!(swept.expired, 1);
    assert!(!f.blob(&id).exists());
    // Cleanup keeps the record, so a late finish is still told it expired.
    assert_eq!(
        wire.id_call(FINISH, &id).await.code(),
        codes::UPLOAD_EXPIRED
    );
    f.advance(30 * DAY);
    assert_eq!(f.uploads.cleanup_pass().unwrap().forgotten, 1);
    assert_eq!(
        wire.id_call(FINISH, &id).await.code(),
        codes::UPLOAD_UNKNOWN
    );
}

#[tokio::test]
async fn a_crash_between_the_blob_sync_and_the_row_update_keeps_a_correct_prefix() {
    let mut f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(3_000);
    let id = wire.begin("a.bin", &file).await;
    wire.chunk(&id, 0, &file[..1000]).await.ok();
    f.uploads
        .crash_after_blob_sync
        .store(true, Ordering::SeqCst);
    let crashed = wire.chunk(&id, 1000, &file[1000..2000]).await;
    assert_eq!(crashed.code(), "provider_unavailable");
    // The bytes reached the blob, but the row never moved past them.
    assert_eq!(std::fs::metadata(f.blob(&id)).unwrap().len(), 2000);
    f.restart();
    let mut wire = f.wire();
    wire.bind_all().await;
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 1000})
    );
    // The unacknowledged tail is not part of the prefix: a shorter retry at the
    // same offset is accepted and the tail is cut off.
    assert_eq!(
        wire.chunk(&id, 1000, &file[1000..1500]).await.ok(),
        json!({"committed": 1500})
    );
    assert_eq!(std::fs::read(f.blob(&id)).unwrap(), file[..1500]);
    wire.chunk(&id, 1500, &file[1500..]).await.ok();
    let finished = wire.id_call(FINISH, &id).await.ok();
    assert_eq!(
        std::fs::read(finished["path"].as_str().unwrap()).unwrap(),
        file
    );
}

#[tokio::test]
async fn an_upload_survives_a_restart_and_a_lost_blob_reads_as_an_empty_prefix() {
    let mut f = Fixture::new();
    let file = content(2_000);
    let id = {
        let mut wire = f.wire();
        wire.bind_all().await;
        let id = wire.begin("a.bin", &file).await;
        wire.chunk(&id, 0, &file[..1000]).await.ok();
        id
    };
    f.restart();
    let mut wire = f.wire();
    wire.bind_all().await;
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 1000})
    );
    // Blobs are not backed up: after a restore the row may outlive its blob.
    std::fs::remove_file(f.blob(&id)).unwrap();
    assert_eq!(
        wire.id_call(STATUS, &id).await.ok(),
        json!({"committed": 0})
    );
    assert_eq!(
        wire.chunk(&id, 1000, &file[1000..]).await.code(),
        codes::OFFSET_GAP
    );
    wire.chunk(&id, 0, &file).await.ok();
    wire.id_call(FINISH, &id).await.ok();
}

#[tokio::test]
async fn a_lost_finish_reply_is_answered_again_with_the_same_path() {
    let mut f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(1_500);
    let id = wire.begin("report.pdf", &file).await;
    wire.chunk(&id, 0, &file).await.ok();
    let first = wire.id_call(FINISH, &id).await.ok();
    assert_eq!(wire.id_call(FINISH, &id).await.ok(), first);
    // Across a restart and a later day, too: the recorded answer, not a new copy.
    f.restart();
    f.advance(29 * DAY);
    let mut wire = f.wire();
    wire.bind_all().await;
    assert_eq!(wire.id_call(FINISH, &id).await.ok(), first);
    let date = Path::new(first["path"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    assert_eq!(listing(&date), ["report.pdf"]);
}

#[tokio::test]
async fn finish_checks_the_size_and_checksum_and_leaves_nothing_on_a_mismatch() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(1_000);
    let incomplete = wire.begin("a.bin", &file).await;
    wire.chunk(&incomplete, 0, &file[..500]).await.ok();
    let refused = wire.id_call(FINISH, &incomplete).await;
    assert_eq!(refused.code(), codes::CHECKSUM_MISMATCH);
    assert_eq!(
        refused.body()["detail"],
        json!({"committed": 500, "size": 1000})
    );

    let mut params = begin_params("b.bin", &file);
    params["sha256"] = json!(sha(b"something else"));
    let id = wire.call(PHONE_ROUTE, BEGIN, params).await.ok()["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    wire.chunk(&id, 0, &file).await.ok();
    assert_eq!(
        wire.id_call(FINISH, &id).await.code(),
        codes::CHECKSUM_MISMATCH
    );
    assert!(
        listing(&f.date_dir()).is_empty(),
        "{:?}",
        listing(&f.date_dir())
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_anywhere_in_the_destination_chain_is_refused() {
    for link in [".cortexkit", ".cortexkit/uploads", "date"] {
        let f = Fixture::new();
        let elsewhere = f.dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let at = match link {
            "date" => f.date_dir(),
            other => f.project.join(other),
        };
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &at).unwrap();
        let mut wire = f.wire();
        wire.bind_all().await;
        let file = content(100);
        let id = wire.begin("a.bin", &file).await;
        wire.chunk(&id, 0, &file).await.ok();
        let refused = wire.id_call(FINISH, &id).await;
        assert_eq!(refused.code(), codes::DESTINATION_UNSAFE, "{link}");
        assert!(
            listing(&elsewhere).is_empty(),
            "{link}: wrote through the link"
        );
        // The upload is intact: once the link is gone it finishes.
        std::fs::remove_file(&at).unwrap();
        wire.id_call(FINISH, &id).await.ok();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn a_symlinked_project_root_is_accepted() {
    let f = Fixture::new();
    let real_project = f.dir.path().join("real-project");
    std::fs::rename(&f.project, &real_project).unwrap();
    std::os::unix::fs::symlink(&real_project, &f.project).unwrap();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(100);
    let id = wire.begin("a.bin", &file).await;
    wire.chunk(&id, 0, &file).await.ok();
    let reply = wire.id_call(FINISH, &id).await.ok();
    let linked = f.date_dir().join("a.bin");
    let resolved = real_project
        .join(".cortexkit/uploads")
        .join(destination::utc_date(START))
        .join("a.bin");
    assert_eq!(Path::new(reply["path"].as_str().unwrap()), linked);
    assert_eq!(std::fs::read(&resolved).unwrap(), file);
    assert_eq!(
        std::fs::canonicalize(linked).unwrap(),
        std::fs::canonicalize(resolved).unwrap()
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn a_final_folder_swapped_for_a_symlink_after_check_is_refused() {
    refuse_symlink_swap(true).await;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn an_intermediate_folder_swapped_for_a_symlink_after_check_is_refused() {
    refuse_symlink_swap(false).await;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn refuse_symlink_swap(final_folder: bool) {
    use destination::test_hook;

    let f = Fixture::new();
    let elsewhere = f.dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let at = if final_folder {
        f.date_dir()
    } else {
        f.project.join(".cortexkit")
    };
    let saved = f.project.join("detached");
    let outside = elsewhere.clone();
    // Populate the symlink target so a path-resolved move would find a source
    // there, rather than fail because the temporary file disappeared.
    test_hook::arm(&f.project, move || {
        std::fs::rename(&at, &saved).unwrap();
        fn copy_tree(from: &Path, to: &Path) {
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    std::fs::create_dir(&target).unwrap();
                    copy_tree(&entry.path(), &target);
                } else {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy_tree(&saved, &outside);
        std::os::unix::fs::symlink(&outside, &at).unwrap();
    });
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(100);
    let id = wire.begin("escape.bin", &file).await;
    wire.chunk(&id, 0, &file).await.ok();
    let refused = wire.id_call(FINISH, &id).await;
    let outside_folder = if final_folder {
        elsewhere
    } else {
        elsewhere.join("uploads").join(destination::utc_date(START))
    };
    assert!(
        !outside_folder.join("escape.bin").exists(),
        "upload escaped the project"
    );
    assert_eq!(
        refused.code(),
        codes::DESTINATION_UNSAFE,
        "final folder: {final_folder}"
    );
    let held_folder = if final_folder {
        f.project.join("detached")
    } else {
        f.project
            .join("detached/uploads")
            .join(destination::utc_date(START))
    };
    assert!(
        !held_folder.join(format!(".{id}.partial")).exists(),
        "temporary cleanup missed the pinned directory"
    );
    assert!(!held_folder.join("escape.bin").exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn a_name_taken_after_listing_is_never_replaced() {
    use destination::test_hook;

    let f = Fixture::new();
    let original = f.date_dir().join("clip.mov");
    let collision = original.clone();
    test_hook::arm(&f.project, move || {
        std::fs::write(collision, b"existing file").unwrap();
    });
    let mut wire = f.wire();
    wire.bind_all().await;
    let bytes = content(100);
    let reply = wire.upload("clip.mov", &bytes).await;
    assert_eq!(std::fs::read(original).unwrap(), b"existing file");
    assert_eq!(
        Path::new(reply["path"].as_str().unwrap()),
        f.date_dir().join("clip-2.mov")
    );
    assert_eq!(
        std::fs::read(reply["path"].as_str().unwrap()).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn a_file_where_a_folder_belongs_is_refused() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.project.join(".cortexkit")).unwrap();
    std::fs::write(f.project.join(".cortexkit/uploads"), b"not a folder").unwrap();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(100);
    let id = wire.begin("a.bin", &file).await;
    wire.chunk(&id, 0, &file).await.ok();
    assert_eq!(
        wire.id_call(FINISH, &id).await.code(),
        codes::DESTINATION_UNSAFE
    );
}

#[tokio::test]
async fn taken_names_get_a_suffix_compared_without_case() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let first = wire.upload("Clip.MOV", &content(10)).await;
    let second = wire.upload("clip.mov", &content(20)).await;
    let third = wire.upload("../CLIP.mov", &content(30)).await;
    let name = |reply: &Value| {
        Path::new(reply["path"].as_str().unwrap())
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(
        [name(&first), name(&second), name(&third)],
        ["Clip.MOV", "clip-2.mov", "CLIP-3.mov"]
    );
}

#[tokio::test]
async fn cleanup_removes_only_unchanged_files_after_thirty_days() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let kept = wire.upload("edited.txt", &content(100)).await;
    let removed = wire.upload("untouched.txt", &content(100)).await;
    let kept = PathBuf::from(kept["path"].as_str().unwrap());
    let removed = PathBuf::from(removed["path"].as_str().unwrap());
    // The user edits one file in place; the size stays the same.
    std::fs::write(&kept, vec![b'x'; 100]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&kept)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
    // Something else in the uploads folder that this module never wrote.
    let stranger = f.date_dir().join("notes.md");
    std::fs::write(&stranger, b"mine").unwrap();
    f.advance(29 * DAY);
    assert_eq!(f.uploads.cleanup_pass().unwrap(), Swept::default());
    assert!(removed.exists());
    f.advance(DAY);
    let swept = f.uploads.cleanup_pass().unwrap();
    assert_eq!(
        (swept.files_removed, swept.files_kept, swept.forgotten),
        (1, 1, 2)
    );
    assert!(!removed.exists());
    assert_eq!(std::fs::read(&kept).unwrap(), vec![b'x'; 100]);
    assert_eq!(std::fs::read(&stranger).unwrap(), b"mine");
    assert!(f.project.join(".cortexkit/uploads/.gitignore").exists());
}

#[tokio::test]
async fn cleanup_removes_blobs_no_unfinished_upload_refers_to() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(10);
    let live = wire.begin("a.bin", &file).await;
    let orphan = f.blob("0000000000000000000000000000beef");
    std::fs::write(&orphan, b"left over").unwrap();
    let other = f.dir.path().join(BLOB_DIR).join("README");
    std::fs::write(&other, b"not a blob").unwrap();
    // Orphan blobs younger than 24 hours are left alone.
    assert_eq!(f.uploads.cleanup_pass().unwrap().orphan_blobs, 0);
    std::fs::File::options()
        .write(true)
        .open(&orphan)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
    assert_eq!(f.uploads.cleanup_pass().unwrap().orphan_blobs, 1);
    assert!(!orphan.exists());
    assert!(other.exists());
    assert!(f.blob(&live).exists());
}

#[tokio::test]
async fn only_the_chunk_takes_a_payload_and_a_malformed_framed_body_is_named() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let body = subc_protocol::framed_body::encode(
        &json!({"method": STATUS, "params": {"upload_id": "00000000000000000000000000000000"}}),
        b"bytes",
    )
    .unwrap();
    let refused = wire.raw(PHONE_ROUTE, body, true).await;
    assert_eq!(refused.code(), "invalid_request");
    // A head length longer than the body.
    let mut malformed = 1_000u32.to_be_bytes().to_vec();
    malformed.extend_from_slice(b"{}");
    let refused = wire.raw(PHONE_ROUTE, malformed, true).await;
    assert_eq!(refused.code(), "invalid_request");
    assert!(
        refused.body()["message"]
            .as_str()
            .unwrap()
            .contains("malformed framed body"),
        "{}",
        refused.body()
    );
    // An unframed chunk is legal and carries no bytes.
    let file = content(10);
    let id = wire.begin("a.bin", &file).await;
    let reply = wire
        .call(PHONE_ROUTE, CHUNK, json!({"upload_id": id, "offset": 0}))
        .await
        .ok();
    assert_eq!(reply, json!({"committed": 0}));
}

#[tokio::test]
async fn a_megabyte_piece_crosses_the_frame_loop_whole() {
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(CHUNK_BYTES as usize * 2 + 7);
    let id = wire.begin("big.bin", &file).await;
    for (index, piece) in file.chunks(CHUNK_BYTES as usize).enumerate() {
        wire.chunk(&id, index as u64 * CHUNK_BYTES, piece)
            .await
            .ok();
    }
    let finished = wire.id_call(FINISH, &id).await.ok();
    assert_eq!(
        std::fs::read(finished["path"].as_str().unwrap()).unwrap(),
        file
    );
}

#[test]
fn the_manifest_declares_the_operations_and_only_the_chunk_payload() {
    let manifest = crate::manifest::manifest().unwrap();
    let json = serde_json::to_value(&manifest).unwrap();
    assert_eq!(
        json["operation_payload"],
        json!({"upload.chunk": {"max_bytes": 8_388_608}})
    );
    // Deserializing runs the protocol's own check of the declaration.
    let _: subc_protocol::manifest::ModuleManifest = serde_json::from_value(json.clone()).unwrap();
    let declared = crate::app_reader::operations();
    for method in METHODS {
        let operation = declared.iter().find(|op| op.name == method);
        assert!(operation.is_some(), "{method} is not declared");
    }
    let text = json.to_string();
    for method in METHODS {
        assert!(text.contains(&format!("\"name\":\"{method}\"")), "{method}");
    }
}

/// Every upload call leaves the app request's log line, as sends and Stop do:
/// the begin under its session, the rest under their upload, and the finish at
/// INFO because it puts a file in the session.
#[tokio::test]
async fn upload_calls_are_logged_like_other_app_requests() {
    let (log, _capture) = crate::app_reader::observe::tests::capture();
    let f = Fixture::new();
    let mut wire = f.wire();
    wire.bind_all().await;
    let file = content(1_500);
    let id = wire.begin("notes.txt", &file).await;
    wire.chunk(&id, 0, &file).await.ok();
    wire.id_call(FINISH, &id).await.ok();
    wire.id_call(STATUS, "0".repeat(32).as_str()).await;
    let text = log.text();
    // The capture writes each event as `level=… field=value …`, one after another.
    let line = |method: &str| {
        text.split("level=")
            .find(|event| event.contains(&format!("method={method} ")))
            .unwrap_or_else(|| panic!("no {method} line: {text}"))
            .to_owned()
    };
    assert!(
        line(BEGIN).contains(&format!("session={SESSION} ")),
        "{text}"
    );
    let finish = line(FINISH);
    assert!(finish.starts_with("INFO "), "{finish}");
    assert!(
        finish.contains(&format!("session=upload:{id} ")),
        "{finish}"
    );
    assert!(finish.contains("outcome=\"ok\""), "{finish}");
    let refused = line(STATUS);
    assert!(refused.starts_with("WARN "), "{refused}");
    assert!(!refused.contains("outcome=\"ok\""), "{refused}");
}
