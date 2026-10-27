use crate::node_manager::auth::LoginResult;
use axum::http::StatusCode;
use lqos_overrides::{CircuitSpeedOverride, OverrideStore};
use lqos_utils::normalize_circuit_id_key;
/// Returns the operator circuit-speed override for one circuit
/// (E3b-adjacent read: the overridden/suspended badge keys off this).
/// Any logged-in session may read.
pub fn get_circuit_speed_override_data(
    _login: LoginResult,
    circuit_id: String,
) -> Result<Option<CircuitSpeedOverride>, StatusCode> {
    let circuit_id = normalize_circuit_id_key(&circuit_id);
    OverrideStore::circuit_speed_override(&circuit_id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Sets or clears the operator circuit-speed override for one circuit
/// (the E3 retarget: admin actions write override entries, NEVER the
/// CSV — the compile folds them in every mode, so UISP/CSV base data
/// stays untouched and an operator rate edit survives integration
/// regeneration by construction).
///
/// `None` = clear the override (revert to plan); `Some` = rate edit.
/// Admin-only; the crate takes the cross-process lock for the whole
/// read-modify-write.
pub fn set_circuit_speed_override_data(
    login: LoginResult,
    circuit_id: String,
    speeds: Option<CircuitSpeedOverride>,
) -> Result<bool, String> {
    if login != LoginResult::Admin {
        return Err("Unauthorized".to_string());
    }
    let circuit_id = normalize_circuit_id_key(&circuit_id);
    OverrideStore::set_circuit_speed_override(&circuit_id, speeds).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_manager::auth::LoginResult;

    // The backend is the authority: write access is Admin-only even
    // though the UI hides the affordances from read-only sessions.
    #[test]
    fn set_circuit_speed_override_rejects_read_only() {
        assert_eq!(
            set_circuit_speed_override_data(
                LoginResult::ReadOnly,
                "c1".to_string(),
                Some(CircuitSpeedOverride::suspend_floor()),
            ),
            Err("Unauthorized".to_string())
        );
        assert_eq!(
            set_circuit_speed_override_data(LoginResult::ReadOnly, "c1".to_string(), None),
            Err("Unauthorized".to_string())
        );
    }
}
