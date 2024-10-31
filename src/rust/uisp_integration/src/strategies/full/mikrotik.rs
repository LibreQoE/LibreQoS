use std::{fs::read_to_string, path::Path};
use lqos_config::Config;
use pyo3::{prepare_freethreaded_python, PyResult, Python};
use crate::uisp_types::Ipv4ToIpv6;

// To ease debugging in the absense of this particular setup, there's a mock function
// available, too.
//
// Enable one of these!
//const PY_FUNC: &str = "pullMikrotikIPv6_Mock";
const PY_FUNC: &str = "pullMikrotikIPv6";


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
    let code = read_to_string(mikrotik_script_path)?;

    // Get the Python environment going
    let mut json_from_python = None;
    prepare_freethreaded_python();
    let result = Python::with_gil(|python| -> PyResult<()> {
        // Run the Python script
        let locals = pyo3::types::PyDict::new(python);
        python.run(&code, None, Some(locals))?;

        // Run the function to pull the Mikrotik data
        let result = python
            .eval(
                &format!("{PY_FUNC}('{}')", mikrotik_dhcp_router_list_path.to_string_lossy()), 
                Some(locals), 
                None
            )?
            .extract::<String>()?;

        // Parse the response.
        // it is an object that looks like this:
        // {
        //   "1.2.3.4" : "2001:db8::1",
        // }
        // We're forcibly returning JSON to make the bridge easier.

        json_from_python = Some(result);
        
        Ok(())
    });

    // If an error occured, fail with as much information as possible
    if let Err(e) = result {
        tracing::error!("Python error: {:?}", e);
        return Err(anyhow::anyhow!("Python error: {:?}", e));
    }

    // If we got this far, we have some JSON to work with
    let json_from_python = json_from_python.unwrap();
    let json = serde_json::from_str::<serde_json::Value>(&json_from_python)?;
    if let Some(map) = json.as_object() {
        let mut result = Vec::new();
        for (ipv4, ipv6) in map {
            result.push(Ipv4ToIpv6 {
                ipv4: ipv4.to_string().replace("\"", ""),
                ipv6: ipv6.to_string().replace("\"", ""),
            });
        }
        return Ok(result);
    } else {
        tracing::error!("Mikrotik data is not an object");
        return Err(anyhow::anyhow!("Mikrotik data is not an object"));
    }
}