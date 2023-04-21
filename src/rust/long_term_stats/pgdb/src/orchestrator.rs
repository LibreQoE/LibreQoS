use crate::{
    hosts::find_emptiest_stats_host, license::StatsHostError,
    organization::does_organization_name_exist,
};
use influxdb2::{
    models::{PostBucketRequest, RetentionRule, Status},
    Client,
};
use sqlx::{Pool, Postgres};
use uuid::Uuid;

pub async fn create_free_trial(
    cnn: Pool<Postgres>,
    organization_name: &str,
) -> Result<String, StatsHostError> {
    // Check that no organization of this name exists already (error if they exist)
    if does_organization_name_exist(cnn.clone(), organization_name).await? {
        return Err(StatsHostError::OrganizationAlreadyExists);
    }

    // Find the most empty, available stats host (error if none)
    let (stats_host_id, influx_url, api_key) = find_emptiest_stats_host(cnn.clone()).await?;

    // Generate a new license key
    let uuid = Uuid::new_v4().to_string();

    // Connect to Influx, and create a new bucket and API token
    create_bucket(&influx_url, &api_key, organization_name).await?;

    // As a transaction:
    //  - Insert into licenses
    //  - Insert into organizations
    let mut tx = cnn.begin().await.map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    sqlx::query("INSERT INTO licenses (key, stats_host) VALUES ($1, $2);")
        .bind(&uuid)
        .bind(stats_host_id)
        .execute(&mut tx)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    sqlx::query("INSERT INTO organizations (key, name, influx_host, influx_org, influx_token, influx_bucket) VALUES ($1, $2, $3, $4, $5, $6);")
        .bind(&uuid)
        .bind(organization_name)
        .bind(&influx_url)
        .bind("LibreQoS")
        .bind(api_key)
        .bind(organization_name)
        .execute(&mut tx)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    
    tx.commit().await.map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    Ok(uuid)
}

async fn create_bucket(
    influx_host: &str,
    api_key: &str,
    org_name: &str,
) -> Result<(), StatsHostError> {
    let influx_url = format!("http://{influx_host}:8086");
    let client = Client::new(influx_url, "LibreQoS", api_key);

    // Is Influx alive and well?
    match client.health().await {
        Err(e) => return Err(StatsHostError::InfluxError(e.to_string())),
        Ok(health) => {
            if health.status == Status::Fail {
                return Err(StatsHostError::InfluxError(
                    "Influx health check failed".to_string(),
                ));
            }
        }
    }

    // Translate the organization name into an id
    let org = client.list_organizations(influxdb2::api::organization::ListOrganizationRequest { 
        descending: None, 
        limit: None, 
        offset: None, 
        org: None, 
        org_id: None, 
        user_id: None 
    }).await.map_err(|e| StatsHostError::InfluxError(e.to_string()))?;
    let org_id = org.orgs[0].id.as_ref().unwrap();

    // Let's make the bucket
    if let Err(e) = client
        .create_bucket(Some(PostBucketRequest {
            org_id: org_id.to_string(),
            name: org_name.to_string(),
            description: None,
            rp: None,
            retention_rules: vec![RetentionRule::new(
                influxdb2::models::retention_rule::Type::Expire,
                604800,
            )], // 1 Week
        }))
        .await
    {
        log::error!("Error creating bucket: {}", e);
        return Err(StatsHostError::InfluxError(e.to_string()));
    }

    Ok(())
}
