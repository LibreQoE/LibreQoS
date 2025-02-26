use std::path::Path;
use std::process::Command;
use lqos_config::Config;
use crate::uisp_types::Ipv4ToIpv6;

pub async fn mikrotik_data(config: &Config) -> anyhow::Result<Vec<Ipv4ToIpv6>> {
    if config.uisp_integration.ipv6_with_mikrotik {
        fetch_mikrotik_data(config).await
    } else {
        Ok(Vec::new())
    }
}

async fn fetch_mikrotik_data(config: &Config) -> anyhow::Result<Vec<Ipv4ToIpv6>> {
    // Find the script and error out if it doesn't exist
    let base_path = Path::new(&config.lqos_directory);
    let mikrotik_script_path = base_path.join("mikrotikFindIPv6.py");
    if !mikrotik_script_path.exists() {
        tracing::error!("Mikrotik script not found at {:?}", mikrotik_script_path);
        return Err(anyhow::anyhow!("Mikrotik script not found at {:?}", mikrotik_script_path));
    }

    // Find the `mikrotikDHCPRouterList.csv` file.
    let mikrotik_dhcp_router_list_path = base_path.join("mikrotikDHCPRouterList.csv");
    if !mikrotik_dhcp_router_list_path.exists() {
        tracing::error!("Mikrotik DHCP router list not found at {:?}", mikrotik_dhcp_router_list_path);
        return Err(anyhow::anyhow!("Mikrotik DHCP router list not found at {:?}", mikrotik_dhcp_router_list_path));
    }

    // Load the script
    let code = mikrotik_script_path.to_string_lossy().to_string();
    let csv_path = mikrotik_dhcp_router_list_path.to_string_lossy().to_string();

    // Get the Python environment going
    let output = Command::new("/usr/bin/python3")
        .args(&[ &code, &csv_path ])
        .output();
    if let Err(e) = output {
        tracing::error!("Python error: {:?}", e);
        return Err(anyhow::anyhow!("Python error: {:?}", e));
    }
    let output = output?;
    let json_from_python = String::from_utf8(output.stdout)?;

    // Parse the JSON

    // If we got this far, we have some JSON to work with
    let json = serde_json::from_str::<serde_json::Value>(&json_from_python)?;
    if let Some(map) = json.as_object() {
        let mut result = Vec::new();
        for (ipv4, ipv6) in map {
            result.push(Ipv4ToIpv6 {
                ipv4: ipv4.to_string().replace("\"", ""),
                ipv6: ipv6.to_string().replace("\"", ""),
            });
        }
        Ok(result)
    } else {
        tracing::error!("Mikrotik data is not an object");
        Err(anyhow::anyhow!("Mikrotik data is not an object"))
    }
}
