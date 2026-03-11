use crate::node_manager::WarningLevel;

pub async fn global_warnings_data() -> Vec<(WarningLevel, String)> {
    crate::node_manager::warnings::get_global_warnings()
}
