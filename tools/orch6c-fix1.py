from pathlib import Path

path = Path("src/hardware_ingest.rs")
text = path.read_text()

anchor = '''    fn request<'a>(root: &'a Path) -> HardwareEvidenceRequest<'a> {
        HardwareEvidenceRequest {
            data_root: root,
            repository: "Memorithm/Test",
            pr_number: 53,
            head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            policy_identity: "abcd1234",
            requirement_id: "jetson-thor-real-device",
        }
    }
'''
if text.count(anchor) != 1:
    raise SystemExit(f"request helper anchor mismatch: {text.count(anchor)}")

insert = anchor + r'''

    fn write_dispatch_and_trust(root: &Path) {
        let dispatch = root.join("config/hardware-dispatch");
        fs::create_dir_all(&dispatch).unwrap();
        fs::write(
            dispatch.join("jetson-thor-real-device.state"),
            "v1\nmode=github_workflow\nrepository=Memorithm/hardware-ci\nworkflow=hardware.yml\nref=main\n",
        )
        .unwrap();
        let trust = root.join("config/hardware-trust");
        fs::create_dir_all(&trust).unwrap();
        fs::write(
            trust.join("jetson-thor-real-device.state"),
            "v1\nsigner_workflow=Memorithm/hardware-ci/.github/workflows/verify.yml\nsigner_digest=cccccccccccccccccccccccccccccccccccccccc\n",
        )
        .unwrap();
    }

    fn canonical_evidence(root: &Path) -> PathBuf {
        root.join("state/hardware-evidence")
            .join(hex_component("Memorithm/Test"))
            .join("pr-53")
            .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .join("jetson-thor-real-device.evidence")
    }

    #[cfg(unix)]
    fn write_fake_gh(root: &Path, token: &str, base_sha: &str, label: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let program = root.join(format!("fake-gh-{label}"));
        let marker = root.join(format!("fake-gh-{label}.log"));
        let script = format!(
            r#"#!/bin/sh
set -eu
printf '%s\n' "$1" >> '{marker}'
case "$1" in
  api)
    test "$2" = 'repos/Memorithm/hardware-ci/actions/artifacts?name=hardware-evidence-{token}&per_page=10'
    test "$3" = --jq
    printf '7\t9\thardware-evidence-{token}\tfalse\t512\n'
    ;;
  run)
    test "$2" = download
    test "$3" = 9
    shift 3
    dir=''
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --repo)
          test "$2" = Memorithm/hardware-ci
          shift 2
          ;;
        --name)
          test "$2" = hardware-evidence-{token}
          shift 2
          ;;
        --dir)
          dir="$2"
          shift 2
          ;;
        *) exit 91 ;;
      esac
    done
    test -n "$dir"
    mkdir -p "$dir"
    cat > "$dir/hardware.evidence" <<'EOF'
v1
repository=Memorithm/Test
pr_number=53
head_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
base_sha={base_sha}
policy_identity=abcd1234
requirement_id=jetson-thor-real-device
result=passed
hardware_class=jetson-thor
device_fingerprint=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
started_at=100
finished_at=101
EOF
    ;;
  attestation)
    test "$2" = verify
    test "$4" = --repo
    test "$5" = Memorithm/Test
    test "$6" = --signer-workflow
    test "$7" = Memorithm/hardware-ci/.github/workflows/verify.yml
    test "$8" = --signer-digest
    test "$9" = cccccccccccccccccccccccccccccccccccccccc
    test "${{10}}" = --source-digest
    test "${{11}}" = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    test "${{12}}" = --predicate-type
    test "${{13}}" = https://slsa.dev/provenance/v1
    ;;
  *) exit 92 ;;
esac
"#,
            marker = marker.display(),
            token = token,
            base_sha = base_sha,
        );
        fs::write(&program, script).unwrap();
        let mut permissions = fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&program, permissions).unwrap();
        (program, marker)
    }

    #[cfg(unix)]
    #[test]
    fn exact_remote_candidate_is_downloaded_verified_imported_and_reverified() {
        let root = temp_root("end-to-end");
        write_dispatch_and_trust(&root);
        let req = request(&root);
        let token = hardware_dispatch::binding_token(&req).unwrap();
        let (program, marker) = write_fake_gh(
            &root,
            &token,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "success",
        );

        let outcome = discover_and_ingest_with_program(&req, &token, program.as_os_str()).unwrap();
        assert!(matches!(
            outcome,
            HardwareIngestOutcome::Imported {
                artifact_id: 7,
                run_id: 9,
                ..
            }
        ));
        let canonical = canonical_evidence(&root);
        assert!(canonical.is_file());
        let evidence = fs::read_to_string(&canonical).unwrap();
        assert!(evidence.contains("base_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
        let actions = fs::read_to_string(&marker).unwrap();
        assert_eq!(actions.lines().collect::<Vec<_>>(), ["api", "run", "attestation", "attestation"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mismatched_remote_manifest_never_reaches_canonical_evidence() {
        let root = temp_root("binding-mismatch");
        write_dispatch_and_trust(&root);
        let req = request(&root);
        let token = hardware_dispatch::binding_token(&req).unwrap();
        let (program, marker) = write_fake_gh(
            &root,
            &token,
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "mismatch",
        );

        let error = discover_and_ingest_with_program(&req, &token, program.as_os_str()).unwrap_err();
        assert!(error.contains("binding mismatch for base_sha"));
        assert!(!canonical_evidence(&root).exists());
        let actions = fs::read_to_string(&marker).unwrap();
        assert_eq!(actions.lines().collect::<Vec<_>>(), ["api", "run"]);
        fs::remove_dir_all(root).unwrap();
    }
'''

text = text.replace(anchor, insert, 1)
path.write_text(text)
print("ORCH6c end-to-end ingest tests added")
