use super::*;
use serde_json::{json, Value};

fn local_launch(session_id: &str) -> LaunchReceipt {
    LaunchReceipt::local("/workspace/swimmers", session_id, false)
}

fn codex_resume(conversation_id: &str) -> AuthorizedProviderResumeReceipt {
    AuthorizedProviderResumeReceipt::resumable(
        ProviderResumeProvider::Codex,
        conversation_id,
        vec![
            "codex".to_string(),
            "resume".to_string(),
            conversation_id.to_string(),
        ],
        format!("codex resume {conversation_id}"),
        ProviderResumeCaptureSource::ProviderResponse,
    )
    .expect("valid provider-authored Codex receipt")
}

#[test]
fn provider_resume_contract_serialization_separates_session_and_conversation_identity() {
    let receipt = AuthorizedProviderResumeLaunchReceipt::new(
        local_launch("swimmers-session-41"),
        codex_resume("thread-92"),
    );

    let value = serde_json::to_value(&receipt).expect("serialize versioned launch receipt");
    assert_eq!(value["version"], PROVIDER_RESUME_LAUNCH_RECEIPT_VERSION);
    assert_eq!(value["session_id"], "swimmers-session-41");
    assert_eq!(value["provider_resume"]["conversation_id"], "thread-92");
    assert_eq!(
        value["provider_resume"]["resume_argv"],
        json!(["codex", "resume", "thread-92"])
    );
    assert_eq!(
        value["provider_resume"]["resume_command"],
        "codex resume thread-92"
    );
    assert_ne!(
        value["session_id"],
        value["provider_resume"]["conversation_id"]
    );

    let round_trip: AuthorizedProviderResumeLaunchReceipt =
        serde_json::from_value(value).expect("deserialize versioned launch receipt");
    assert_eq!(round_trip, receipt);
    assert!(round_trip.provider_resume().is_resumable());
    assert_eq!(
        round_trip.provider_resume().resume_argv(),
        Some(
            ["codex", "resume", "thread-92"]
                .map(str::to_string)
                .as_slice()
        )
    );
}

#[test]
fn provider_resume_contract_legacy_launch_receipt_defaults_to_explicit_unknown() {
    let legacy = serde_json::to_value(local_launch("legacy-session"))
        .expect("serialize pre-extension launch receipt");
    assert!(legacy.get("version").is_none());
    assert!(legacy.get("provider_resume").is_none());

    let upgraded: AuthorizedProviderResumeLaunchReceipt =
        serde_json::from_value(legacy.clone()).expect("legacy receipt remains readable");
    assert_eq!(upgraded.version(), PROVIDER_RESUME_LAUNCH_RECEIPT_VERSION);
    assert_eq!(
        upgraded.provider_resume().capability(),
        ProviderResumeCapability::Unknown
    );
    assert!(!upgraded.provider_resume().is_resumable());

    let additive = serde_json::to_value(&upgraded).expect("serialize additive receipt");
    let legacy_reader: LaunchReceipt =
        serde_json::from_value(additive).expect("legacy reader ignores additive fields");
    assert_eq!(legacy_reader, local_launch("legacy-session"));
}

#[test]
fn provider_resume_contract_unknown_and_unsupported_disable_resume_claim() {
    let unknown = AuthorizedProviderResumeReceipt::unknown(
        ProviderResumeProvider::Codex,
        ProviderResumeCaptureSource::ProviderResponse,
    );
    let unsupported = AuthorizedProviderResumeReceipt::unsupported(ProviderResumeProvider::Claude);

    for receipt in [unknown, unsupported] {
        assert!(!receipt.is_resumable());
        assert_eq!(receipt.conversation_id(), None);
        assert_eq!(receipt.resume_argv(), None);
        assert_eq!(receipt.resume_command(), None);
    }

    let missing_identity = json!({
        "provider": "codex",
        "capability": "resumable",
        "capture_source": "provider_response",
        "resume_argv": ["codex", "resume", "thread-missing"],
        "resume_command": "codex resume thread-missing"
    });
    let error = serde_json::from_value::<AuthorizedProviderResumeReceipt>(missing_identity)
        .expect_err("missing conversation identity must fail closed");
    assert!(error.to_string().contains("requires conversation_id"));

    let leaked_identity = json!({
        "provider": "codex",
        "capability": "unknown",
        "capture_source": "provider_response",
        "conversation_id": "thread-must-not-claim",
        "resume_argv": ["codex", "resume", "thread-must-not-claim"],
        "resume_command": "codex resume thread-must-not-claim"
    });
    let error = serde_json::from_value::<AuthorizedProviderResumeReceipt>(leaked_identity)
        .expect_err("unknown capability with identity must fail closed");
    assert!(error.to_string().contains("cannot carry resume identity"));
}

#[test]
fn provider_resume_contract_default_and_public_json_omit_private_identity_and_controls() {
    let receipt = AuthorizedProviderResumeLaunchReceipt::new(
        local_launch("private-swimmers-id"),
        codex_resume("thread-public"),
    );
    let public = serde_json::to_value(receipt.public_projection())
        .expect("serialize phone-safe provider projection");

    assert_eq!(public["provider"], "codex");
    assert_eq!(
        public["capability"], "unknown",
        "identity-free public projection must not claim resumability"
    );
    let forbidden = [
        "conversation_id",
        "resume_command",
        "session_id",
        "swimmers_session_id",
        "remote_session_id",
        "resume_argv",
        "tmux_name",
        "tmux_session",
        "pane_id",
        "pid",
        "pgid",
        "generation",
        "session_generation",
        "control_handle",
        "cleanup_argv",
    ];
    let default = serde_json::to_value(ProviderResumePublicProjection::default())
        .expect("serialize default provider projection");
    assert_no_forbidden_keys(&default, &forbidden);
    assert_no_forbidden_keys(&public, &forbidden);
}

#[test]
fn provider_resume_contract_parallel_receipts_preserve_authoritative_pairing() {
    let receipts: Vec<_> = (0..64)
        .map(|index| {
            let swimmers_id = format!("swimmers-{index}");
            let conversation_id = format!("thread-{index}");
            AuthorizedProviderResumeLaunchReceipt::new(
                local_launch(&swimmers_id),
                codex_resume(&conversation_id),
            )
        })
        .collect();

    for (index, receipt) in receipts.iter().enumerate() {
        assert_eq!(
            receipt.launch().session_id.as_deref(),
            Some(format!("swimmers-{index}").as_str())
        );
        assert_eq!(
            receipt.provider_resume().conversation_id(),
            Some(format!("thread-{index}").as_str())
        );
    }
}

fn assert_no_forbidden_keys(value: &Value, forbidden: &[&str]) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                assert!(
                    !forbidden.contains(&key.as_str()),
                    "public projection leaked forbidden key {key}"
                );
                assert_no_forbidden_keys(value, forbidden);
            }
        }
        Value::Array(values) => {
            for value in values {
                assert_no_forbidden_keys(value, forbidden);
            }
        }
        _ => {}
    }
}
