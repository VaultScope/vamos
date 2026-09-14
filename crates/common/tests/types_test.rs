use vaultscope_common::types::*;

#[test]
fn test_power_action_serialization() {
    let actions = [
        (PowerAction::Start, "\"start\""),
        (PowerAction::Stop, "\"stop\""),
        (PowerAction::Restart, "\"restart\""),
        (PowerAction::Reset, "\"reset\""),
    ];
    for (action, expected) in actions {
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, expected);
    }
}

#[test]
fn test_power_action_deserialization() {
    let start: PowerAction = serde_json::from_str("\"start\"").unwrap();
    assert_eq!(start, PowerAction::Start);

    let stop: PowerAction = serde_json::from_str("\"stop\"").unwrap();
    assert_eq!(stop, PowerAction::Stop);
}

#[test]
fn test_power_action_invalid_deserialization() {
    let result = serde_json::from_str::<PowerAction>("\"invalid\"");
    assert!(result.is_err());
}

#[test]
fn test_resource_status_serialization() {
    let statuses = [
        (ResourceStatus::Running, "\"running\""),
        (ResourceStatus::Stopped, "\"stopped\""),
        (ResourceStatus::Starting, "\"starting\""),
        (ResourceStatus::Deleting, "\"deleting\""),
        (ResourceStatus::Unknown, "\"unknown\""),
    ];
    for (status, expected) in statuses {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, expected);
    }
}

#[test]
fn test_resource_status_roundtrip() {
    let status = ResourceStatus::Running;
    let json = serde_json::to_string(&status).unwrap();
    let deserialized: ResourceStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(
        serde_json::to_string(&deserialized).unwrap(),
        serde_json::to_string(&status).unwrap()
    );
}

#[test]
fn test_provision_params_serialization() {
    let params = ProvisionParams {
        server_type: "cx21".to_string(),
        location: "nbg1".to_string(),
        image: "ubuntu-22.04".to_string(),
        ssh_keys: vec!["key1".to_string(), "key2".to_string()],
        name: "test-server".to_string(),
        extra: serde_json::json!({"label": "test"}),
    };

    let json = serde_json::to_string(&params).unwrap();
    let deserialized: ProvisionParams = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.server_type, "cx21");
    assert_eq!(deserialized.location, "nbg1");
    assert_eq!(deserialized.image, "ubuntu-22.04");
    assert_eq!(deserialized.ssh_keys.len(), 2);
    assert_eq!(deserialized.name, "test-server");
}

#[test]
fn test_provision_result_serialization() {
    let result = ProvisionResult {
        resource_id: "srv-12345".to_string(),
        ip: "1.2.3.4".to_string(),
        hostname: "my-server".to_string(),
        raw_response: serde_json::json!({"id": 12345}),
    };

    let json = serde_json::to_string(&result).unwrap();
    let deserialized: ProvisionResult = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.resource_id, "srv-12345");
    assert_eq!(deserialized.ip, "1.2.3.4");
    assert_eq!(deserialized.hostname, "my-server");
}

#[test]
fn test_reinstall_params_serialization() {
    let params = ReinstallParams {
        image: "debian-12".to_string(),
        ssh_keys: vec!["new-key".to_string()],
    };

    let json = serde_json::to_string(&params).unwrap();
    assert!(json.contains("debian-12"));
    assert!(json.contains("new-key"));

    let deserialized: ReinstallParams = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.image, "debian-12");
    assert_eq!(deserialized.ssh_keys, vec!["new-key".to_string()]);
}
