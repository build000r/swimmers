//! Production-serde fixtures for Cass two-stage admission.

use std::fs;
use std::path::PathBuf;

use swimmers::types::{
    CassAdmissionIntent, CassAdmissionReservationEnvelope, CassAdmissionSubject, CassOrigin,
    CassProviderIdentity, CASS_ADMISSION_INTENT_SCHEMA, CASS_ADMISSION_RESERVATION_SCHEMA,
    CASS_ADMISSION_SUBJECT_SCHEMA, CASS_PROVIDER_IDENTITY_SCHEMA,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn read(name: &str) -> String {
    fs::read_to_string(fixture(name)).unwrap_or_else(|error| panic!("read {name}: {error}"))
}

fn main() {
    let local = CassProviderIdentity::from_json_str(&read("cass_provider_identity_v1_local.json"))
        .expect("local identity");
    assert_eq!(local.schema_version, CASS_PROVIDER_IDENTITY_SCHEMA);
    assert_eq!(local.provider, "codex");
    assert_eq!(local.origin, CassOrigin::Local);
    assert_eq!(local.batch_index, 0);

    let remote =
        CassProviderIdentity::from_json_str(&read("cass_provider_identity_v1_remote.json"))
            .expect("remote identity");
    assert_eq!(remote.origin, CassOrigin::Remote);
    assert_eq!(remote.batch_index, 1);

    let secret_err =
        CassProviderIdentity::from_json_str(&read("cass_provider_identity_v1_reject_secret.json"))
            .expect_err("secret identity");
    assert_eq!(secret_err.code, "secret_shaped_key");

    let extra_err =
        CassProviderIdentity::from_json_str(&read("cass_provider_identity_v1_reject_extra.json"))
            .expect_err("extra identity");
    assert_eq!(extra_err.code, "document_invalid");

    let intent = CassAdmissionIntent::from_json_str(&read("cass_admission_intent_v1.json"))
        .expect("admission intent");
    assert_eq!(intent.schema_version, CASS_ADMISSION_INTENT_SCHEMA);
    assert!(!intent.provider.is_empty());

    let session_err =
        CassAdmissionIntent::from_json_str(&read("cass_admission_intent_v1_reject_session.json"))
            .expect_err("intent with provider session UUID");
    assert_eq!(session_err.code, "document_invalid");
    assert!(session_err
        .message
        .contains("must not carry a provider session UUID"));

    let reservation = CassAdmissionReservationEnvelope::from_value(
        &serde_json::from_str(&read("cass_admission_intent_v1_reservation.json")).unwrap(),
    )
    .expect("reservation envelope");
    assert_eq!(
        reservation.schema_version,
        CASS_ADMISSION_RESERVATION_SCHEMA
    );
    assert!(reservation.reservation_id.starts_with("rsv_"));
    assert!(!reservation.reservation_id.contains("Bearer "));

    let subject = CassAdmissionSubject::from_value(
        &serde_json::from_str(&read("cass_provider_identity_v1_refined.json")).unwrap(),
    )
    .expect("refined subject");
    assert_eq!(subject.schema_version, CASS_ADMISSION_SUBJECT_SCHEMA);
    assert_eq!(subject.adapter_id, "codex-jsonl-v1");

    println!("cass_provider_identity fixtures ok");
}
