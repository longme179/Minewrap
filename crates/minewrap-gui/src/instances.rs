//! Instance-list state mapping (daemon `combined_state` → UI dot/label).

/// UI state codes (Slint `InstanceRow.state`).
pub const ONLINE: i32 = 0;
pub const STARTING: i32 = 1;
pub const STOPPING: i32 = 2;
pub const OFFLINE: i32 = 3;
pub const CRASHED: i32 = 4;
pub const UNRESPONSIVE: i32 = 5;
pub const UNKNOWN: i32 = 6;

/// Map the daemon's `combined_state` string to a UI code.
pub fn state_code(combined_state: &str) -> i32 {
    match combined_state {
        "online" => ONLINE,
        "starting" => STARTING,
        "stopping" => STOPPING,
        "offline" => OFFLINE,
        "crashed" => CRASHED,
        "unresponsive" => UNRESPONSIVE,
        _ => UNKNOWN,
    }
}

/// Label rendered under the instance name.
pub fn state_label(code: i32) -> &'static str {
    match code {
        ONLINE => "online",
        STARTING => "starting",
        STOPPING => "stopping",
        OFFLINE => "offline",
        CRASHED => "crashed",
        UNRESPONSIVE => "unresponsive",
        _ => "unknown",
    }
}

/// Start is meaningful from a stopped/crashed state only.
pub fn can_start(code: i32) -> bool {
    matches!(code, OFFLINE | CRASHED | UNKNOWN)
}

/// Stop is meaningful while a process exists (incl. a stuck one).
pub fn can_stop(code: i32) -> bool {
    matches!(code, ONLINE | STARTING | STOPPING | UNRESPONSIVE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_codes_cover_every_daemon_state() {
        assert_eq!(state_code("online"), ONLINE);
        assert_eq!(state_code("starting"), STARTING);
        assert_eq!(state_code("stopping"), STOPPING);
        assert_eq!(state_code("offline"), OFFLINE);
        assert_eq!(state_code("crashed"), CRASHED);
        assert_eq!(state_code("unresponsive"), UNRESPONSIVE);
        assert_eq!(state_code("something-new"), UNKNOWN);
    }

    #[test]
    fn labels_round_trip() {
        for (s, code) in [
            ("online", ONLINE),
            ("starting", STARTING),
            ("stopping", STOPPING),
            ("offline", OFFLINE),
            ("crashed", CRASHED),
            ("unresponsive", UNRESPONSIVE),
        ] {
            assert_eq!(state_label(state_code(s)), s);
            assert_eq!(code, state_code(s));
        }
    }

    #[test]
    fn button_applicability_follows_state() {
        assert!(can_start(state_code("offline")));
        assert!(can_start(state_code("crashed")));
        assert!(!can_start(state_code("online")));
        assert!(!can_start(state_code("starting")));
        assert!(can_stop(state_code("online")));
        assert!(can_stop(state_code("unresponsive")));
        assert!(!can_stop(state_code("offline")));
    }
}
