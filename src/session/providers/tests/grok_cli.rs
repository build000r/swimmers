use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::types::{LaunchReceipt, ProviderResumeCaptureSource, ProviderResumeProvider};

use super::*;

const PARALLEL_LAUNCHES: usize = 64;
const COLLISION_EXIT_CODE: i32 = 73;
static FIXTURE_EXECUTION_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn grok_cli_provider_parallel_launches_preassign_unique_receipt_bound_uuids() {
    let _fixture_guard = fixture_execution_guard();
    let fixture = FakeGrok::new();
    let provider = Arc::new(
        GrokCliProvider::with_program(fixture.program())
            .expect("construct provider from fake Grok path"),
    );
    let cwd = fixture.cwd().to_path_buf();

    let receipts = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..PARALLEL_LAUNCHES)
            .map(|index| {
                let provider = Arc::clone(&provider);
                let cwd = cwd.clone();
                scope.spawn(move || {
                    let plan = provider
                        .prepare_new_conversation(&cwd)
                        .expect("prepare preassigned Grok launch");
                    let expected_id = plan.conversation_id().to_string();
                    let argv = plan.argv();
                    let success = provider
                        .execute_to_exit(&plan)
                        .expect("fake Grok accepts unique UUID");
                    let receipt = success
                        .bind_receipt(LaunchReceipt::local(
                            cwd.to_string_lossy(),
                            format!("swimmers-{index}"),
                            false,
                        ))
                        .expect("bind matching cwd and provider identity");
                    (expected_id, argv, receipt)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("parallel launch thread"))
            .collect::<Vec<_>>()
    });

    let ids: HashSet<_> = receipts
        .iter()
        .map(|(id, _, receipt)| {
            let parsed = uuid::Uuid::parse_str(id).expect("preassigned ID is UUID");
            assert_eq!(
                receipt.provider_resume().provider(),
                ProviderResumeProvider::Grok
            );
            assert_eq!(
                receipt.provider_resume().capture_source(),
                ProviderResumeCaptureSource::Preassigned
            );
            assert_eq!(
                receipt.provider_resume().conversation_id(),
                Some(id.as_str())
            );
            assert_eq!(
                receipt.launch().local_cwd.as_deref(),
                Some(cwd.to_string_lossy().as_ref())
            );
            parsed
        })
        .collect();
    assert_eq!(ids.len(), PARALLEL_LAUNCHES);

    for (id, argv, receipt) in &receipts {
        assert_eq!(argv[1], OsString::from("--session-id"));
        assert_eq!(argv[2], OsString::from(id));
        assert_eq!(
            receipt.provider_resume().resume_argv(),
            Some(
                [
                    fixture.program().to_string_lossy().into_owned(),
                    "--resume".to_string(),
                    id.clone(),
                ]
                .as_slice()
            )
        );
    }

    let events = fixture.events();
    assert_eq!(
        events
            .lines()
            .filter(|line| line.starts_with("new "))
            .count(),
        PARALLEL_LAUNCHES
    );
}

#[test]
fn grok_cli_provider_collision_and_rejection_fail_without_fallback_receipt() {
    let _fixture_guard = fixture_execution_guard();
    let fixture = FakeGrok::new();
    let provider = GrokCliProvider::with_program(fixture.program())
        .expect("construct provider from fake Grok path");
    let plan = provider
        .prepare_new_conversation(fixture.cwd())
        .expect("prepare launch");

    let first = provider
        .execute_to_exit(&plan)
        .expect("first use of UUID succeeds");
    let collision = provider
        .execute_to_exit(&plan)
        .expect_err("reusing preassigned UUID must fail closed");
    assert!(matches!(
        collision,
        GrokCliProviderError::Rejected {
            operation: "launch",
            code: Some(COLLISION_EXIT_CODE)
        }
    ));

    let receipt = first
        .bind_receipt(LaunchReceipt::local(
            fixture.cwd().to_string_lossy(),
            "swimmers-collision",
            false,
        ))
        .expect("only successful launch can bind receipt");
    assert_eq!(
        receipt.provider_resume().conversation_id(),
        Some(plan.conversation_id())
    );

    let events = fixture.events();
    let id_events: Vec<_> = events
        .lines()
        .filter(|line| line.contains(plan.conversation_id()))
        .collect();
    assert_eq!(id_events.len(), 2);
    assert!(id_events[0].starts_with("new "));
    assert!(id_events[1].starts_with("collision "));

    let rejecting = RejectingGrok::new(29);
    let rejecting_provider =
        GrokCliProvider::with_program(rejecting.program()).expect("construct rejecting provider");
    let rejected_plan = rejecting_provider
        .prepare_new_conversation(rejecting.cwd())
        .expect("prepare rejected launch");
    assert!(matches!(
        rejecting_provider.execute_to_exit(&rejected_plan),
        Err(GrokCliProviderError::Rejected {
            operation: "launch",
            code: Some(29)
        })
    ));
    assert_eq!(
        rejecting.invocations(),
        1,
        "adapter must not retry without --session-id"
    );
}

#[test]
fn grok_cli_provider_failed_spawn_and_conflicting_controls_fail_closed() {
    let _fixture_guard = fixture_execution_guard();
    let fixture = tempfile::tempdir().expect("tempdir");
    let missing_program = fixture.path().join("missing-grok");
    let provider =
        GrokCliProvider::with_program(&missing_program).expect("valid missing executable path");
    let plan = provider
        .prepare_new_conversation(fixture.path())
        .expect("prepare launch without touching executable");
    assert!(matches!(
        provider.execute_to_exit(&plan),
        Err(GrokCliProviderError::Spawn {
            operation: "launch",
            ..
        })
    ));

    let real_provider =
        GrokCliProvider::with_program("/bin/true").expect("construct /bin/true provider");
    for forbidden in [
        "--session-id",
        "--session-id=other",
        "-s",
        "--resume",
        "--resume=other",
        "-r",
        "--continue",
        "-c",
        "--fork-session",
        "--cwd",
        "--cwd=/other",
    ] {
        assert!(matches!(
            real_provider.prepare_launch(fixture.path(), [OsString::from(forbidden)]),
            Err(GrokCliProviderError::ConflictingControlArgument)
        ));
    }
}

#[test]
fn grok_cli_provider_receipt_requires_exact_cwd_provider_and_cli_id() {
    let _fixture_guard = fixture_execution_guard();
    let fixture = FakeGrok::new();
    let provider = GrokCliProvider::with_program(fixture.program())
        .expect("construct provider from fake Grok path");
    let plan = provider
        .prepare_launch(
            fixture.cwd(),
            [
                OsString::from("--always-approve"),
                OsString::from("--no-alt-screen"),
            ],
        )
        .expect("prepare launch with non-identity arguments");
    assert_eq!(plan.cwd(), fixture.cwd());
    assert!(plan.display_command().contains("--session-id"));

    let success = provider
        .verify_observed_launch(
            &plan,
            GrokLaunchObservation::running(plan.argv(), plan.cwd()),
        )
        .expect("exact running tmux observation verifies without a second launch");
    let wrong_cwd = fixture.cwd().join("other");
    let error = success
        .bind_receipt(LaunchReceipt::local(
            wrong_cwd.to_string_lossy(),
            "swimmers-wrong-cwd",
            false,
        ))
        .expect_err("receipt cwd must match executed cwd");
    assert!(matches!(
        error,
        GrokCliProviderError::ReceiptCwdMismatch { .. }
    ));

    let mismatched_argv = provider.verify_observed_launch(
        &plan,
        GrokLaunchObservation::running(
            [
                fixture.program().as_os_str().to_os_string(),
                OsString::from("--session-id"),
                OsString::from(uuid::Uuid::new_v4().to_string()),
                OsString::from("--always-approve"),
                OsString::from("--no-alt-screen"),
            ]
            .to_vec(),
            plan.cwd(),
        ),
    );
    assert!(matches!(
        mismatched_argv,
        Err(GrokCliProviderError::ObservedArgvMismatch)
    ));

    let mismatched_cwd = provider.verify_observed_launch(
        &plan,
        GrokLaunchObservation::running(plan.argv(), fixture.cwd().join("other")),
    );
    assert!(matches!(
        mismatched_cwd,
        Err(GrokCliProviderError::ObservedCwdMismatch { .. })
    ));

    let success = provider
        .execute_to_exit(&plan)
        .expect("fake Grok accepts exact prepared UUID");
    let receipt = success
        .bind_receipt(LaunchReceipt::local(
            fixture.cwd().to_string_lossy(),
            "swimmers-exact",
            false,
        ))
        .expect("bind exact observed provider/cwd/ID");
    assert_eq!(
        receipt.provider_resume().conversation_id(),
        Some(plan.conversation_id())
    );

    let log_line = fixture
        .events()
        .lines()
        .find(|line| line.starts_with("new "))
        .expect("new conversation log")
        .to_string();
    let fields: Vec<_> = log_line.splitn(3, ' ').collect();
    assert_eq!(fields[1], plan.conversation_id());
    assert_eq!(fields[2], fixture.cwd().to_string_lossy());
}

#[test]
fn grok_cli_provider_resumes_exact_conversation_after_tmux_cleanup() {
    let _fixture_guard = fixture_execution_guard();
    let fixture = FakeGrok::new();
    let provider = GrokCliProvider::with_program(fixture.program())
        .expect("construct provider from fake Grok path");
    let plan = provider
        .prepare_new_conversation(fixture.cwd())
        .expect("prepare launch");
    let conversation_id = plan.conversation_id().to_string();
    let success = provider
        .execute_to_exit(&plan)
        .expect("fake Grok persists conversation");
    let receipt = success
        .bind_receipt(LaunchReceipt::local(
            fixture.cwd().to_string_lossy(),
            "swimmers-session-removed-next",
            false,
        ))
        .expect("bind launch receipt");

    let tmux_marker = fixture.cwd().join("tmux-session.marker");
    fs::write(&tmux_marker, "alive").expect("write tmux marker");
    fs::remove_file(&tmux_marker).expect("simulate exact tmux session cleanup");
    assert!(!tmux_marker.exists());

    provider
        .resume_to_exit(receipt.provider_resume(), fixture.cwd())
        .expect("resume persisted Grok conversation after tmux cleanup");

    let events = fixture.events();
    assert!(
        events
            .lines()
            .any(|line| line == format!("resume {conversation_id}")),
        "resume must target exact preassigned UUID; events={events:?}"
    );
}

struct FakeGrok {
    _temp: tempfile::TempDir,
    program: PathBuf,
    cwd: PathBuf,
    events: PathBuf,
}

impl FakeGrok {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("fake Grok tempdir");
        let program = temp.path().join("grok");
        let cwd = temp.path().join("workspace");
        let state = temp.path().join("sessions");
        let events = temp.path().join("events.log");
        fs::create_dir_all(&cwd).expect("fake cwd");
        fs::create_dir_all(&state).expect("fake session state");
        write_executable(
            &program,
            &format!(
                "#!/bin/sh\n\
                 state={}\n\
                 events={}\n\
                 op=\"$1\"\n\
                 id=\"$2\"\n\
                 case \"$op\" in\n\
                   --session-id)\n\
                     if (set -C; : > \"$state/$id\") 2>/dev/null; then\n\
                       printf 'new %s %s\\n' \"$id\" \"$PWD\" >> \"$events\"\n\
                       exit 0\n\
                     fi\n\
                     printf 'collision %s\\n' \"$id\" >> \"$events\"\n\
                     exit {COLLISION_EXIT_CODE}\n\
                     ;;\n\
                   --resume)\n\
                     if [ -f \"$state/$id\" ]; then\n\
                       printf 'resume %s\\n' \"$id\" >> \"$events\"\n\
                       exit 0\n\
                     fi\n\
                     printf 'missing %s\\n' \"$id\" >> \"$events\"\n\
                     exit 74\n\
                     ;;\n\
                   *) exit 75 ;;\n\
                 esac\n",
                crate::launcher::shell_single_quote(&state.to_string_lossy()),
                crate::launcher::shell_single_quote(&events.to_string_lossy()),
            ),
        );
        Self {
            _temp: temp,
            program,
            cwd,
            events,
        }
    }

    fn program(&self) -> &Path {
        &self.program
    }

    fn cwd(&self) -> &Path {
        &self.cwd
    }

    fn events(&self) -> String {
        fs::read_to_string(&self.events).unwrap_or_default()
    }
}

struct RejectingGrok {
    _temp: tempfile::TempDir,
    program: PathBuf,
    cwd: PathBuf,
    invocations: PathBuf,
}

impl RejectingGrok {
    fn new(exit_code: i32) -> Self {
        let temp = tempfile::tempdir().expect("rejecting Grok tempdir");
        let program = temp.path().join("grok");
        let cwd = temp.path().join("workspace");
        let invocations = temp.path().join("invocations.log");
        fs::create_dir_all(&cwd).expect("rejecting cwd");
        write_executable(
            &program,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexit {exit_code}\n",
                crate::launcher::shell_single_quote(&invocations.to_string_lossy())
            ),
        );
        Self {
            _temp: temp,
            program,
            cwd,
            invocations,
        }
    }

    fn program(&self) -> &Path {
        &self.program
    }

    fn cwd(&self) -> &Path {
        &self.cwd
    }

    fn invocations(&self) -> usize {
        fs::read_to_string(&self.invocations)
            .unwrap_or_default()
            .lines()
            .count()
    }
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write fake executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .expect("chmod fake executable");
    }
}

fn fixture_execution_guard() -> std::sync::MutexGuard<'static, ()> {
    FIXTURE_EXECUTION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
