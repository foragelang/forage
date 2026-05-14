//! Studio-side hub_sync surface: deeplink parsing + typed publish
//! error mapping. The wire-shape integration coverage lives in the
//! `forage-hub` crate; here we just exercise the bits that are
//! Studio-specific (URL handler validation, the `PublishError`
//! discriminant the UI consumes).

use forage_hub::HubError;
use forage_studio_lib::hub_sync::{PublishError, parse_clone_url, validate_segments};

#[test]
fn deeplink_clone_round_trips() {
    let (a, s, v) = parse_clone_url("forage://clone/alice/zen-leaf").unwrap();
    assert_eq!(a, "alice");
    assert_eq!(s, "zen-leaf");
    assert!(v.is_none());

    let (_, _, v) = parse_clone_url("forage://clone/alice/zen-leaf?version=4").unwrap();
    assert_eq!(v, Some(4));
}

#[test]
fn deeplink_clone_rejects_hostile_segments() {
    // The hub's SEGMENT_RE rejects uppercase, leading dashes, dots,
    // path traversal, etc. The Studio handler runs the same check
    // *before* passing the segments to sync_from_hub so an
    // opportunistic deeplink can't smuggle anything past it.
    for bad in [
        "forage://clone/Alice/zen-leaf",
        "forage://clone/../zen-leaf",
        "forage://clone/alice/zen leaf",
        "forage://clone/alice/.hidden",
        "forage://clone/alice/zen-leaf/extra",  // too many segments
        "forage://other-verb/alice/zen-leaf",
        "https://example.com/clone/alice/zen-leaf",
    ] {
        assert!(parse_clone_url(bad).is_err(), "should reject: {bad}");
    }
}

#[test]
fn segment_validation_matches_hub_regex() {
    // Greedy validation: lowercase alphanumeric + hyphens, up to 39
    // characters, must start with [a-z0-9]. Mirrors the hub-api's
    // SEGMENT_RE.
    assert!(validate_segments("alice", "zen-leaf").is_ok());
    assert!(validate_segments("a", "b").is_ok());
    assert!(validate_segments("alice", "Zen-Leaf").is_err());
    assert!(validate_segments("-alice", "zen-leaf").is_err());
    assert!(validate_segments("", "zen-leaf").is_err());
    assert!(validate_segments("alice", "zen.leaf").is_err());
}

#[test]
fn stale_base_hub_error_maps_to_typed_publish_error() {
    // The Tauri command path returns PublishError to the UI; the
    // stale-base discriminant carries the version numbers the UI
    // needs to render the rebase banner.
    let typed = PublishError::from_hub_error(HubError::StaleBase {
        latest_version: 5,
        your_base: Some(3),
        message: "rebase to v5 and retry".into(),
    });
    match typed {
        PublishError::StaleBase {
            latest_version,
            your_base,
            message,
        } => {
            assert_eq!(latest_version, 5);
            assert_eq!(your_base, Some(3));
            assert!(message.contains("rebase"));
        }
        other => panic!("expected StaleBase, got {other:?}"),
    }
}

#[test]
fn generic_hub_error_maps_to_other_publish_error() {
    let typed = PublishError::from_hub_error(HubError::Api {
        status: 500,
        code: "internal".into(),
        message: "server exploded".into(),
    });
    match typed {
        PublishError::Other { message } => assert!(message.contains("internal")),
        other => panic!("expected Other, got {other:?}"),
    }
}

#[test]
fn typed_publish_error_serializes_with_discriminant() {
    // The UI dispatches off the `kind` tag — assert it's there
    // (snake-cased) and that the variant payload rides through.
    let v = PublishError::StaleBase {
        latest_version: 5,
        your_base: Some(3),
        message: "x".into(),
    };
    let j = serde_json::to_value(&v).unwrap();
    assert_eq!(j["kind"], "stale_base");
    assert_eq!(j["latest_version"], 5);
    assert_eq!(j["your_base"], 3);

    let v = PublishError::NotSignedIn {
        message: "log in".into(),
    };
    let j = serde_json::to_value(&v).unwrap();
    assert_eq!(j["kind"], "not_signed_in");
    assert_eq!(j["message"], "log in");
}
