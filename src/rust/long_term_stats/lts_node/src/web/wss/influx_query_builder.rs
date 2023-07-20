#![allow(dead_code)]
use influxdb2::{Client, models::Query};
use influxdb2_structmap::FromMap;
use pgdb::{sqlx::{Pool, Postgres}, organization_cache::get_org_details, OrganizationDetails};
use anyhow::{Result, Error};
use tracing::instrument;
use super::queries::time_period::InfluxTimePeriod;

#[derive(Debug)]
pub struct InfluxQueryBuilder {
    imports: Vec<String>,
    fields: Vec<String>,
    period: InfluxTimePeriod,
    measurement: Option<String>,
    group_by: Vec<String>,
    aggregate_window: bool,
    yield_as: Option<String>,
    host_id: Option<String>,
}

impl InfluxQueryBuilder {
    pub fn new(period: InfluxTimePeriod) -> Self {
        Self {
            fields: Vec::new(),
            imports: Vec::new(),
            group_by: Vec::new(),
            period,
            measurement: None,
            aggregate_window: true,
            yield_as: Some("last".to_string()),
            host_id: None,
        }
    }

    pub fn with_measurement<S: ToString>(mut self, measurement: S) -> Self {
        self.measurement = Some(measurement.to_string());
        self
    }

    pub fn with_import<S: ToString>(mut self, import: S) -> Self {
        self.imports.push(import.to_string());
        self
    }

    pub fn with_field<S: ToString>(mut self, field: S) -> Self {
        self.fields.push(field.to_string());
        self
    }

    pub fn with_fields<S: ToString>(mut self, fields: &[S]) -> Self {
        for field in fields.iter() {
            self.fields.push(field.to_string());
        }
        self
    }

    pub fn with_group<S: ToString>(mut self, group: S) -> Self {
        self.group_by.push(group.to_string());
        self
    }

    pub fn with_groups<S: ToString>(mut self, group: &[S]) -> Self {
        for group in group.iter() {
            self.group_by.push(group.to_string());
        }
        self
    }

    pub fn with_host_id<S: ToString>(mut self, host_id: S) -> Self {
        self.host_id = Some(host_id.to_string());
        self
    }

    pub fn sample_no_window(mut self) -> Self {
        self.aggregate_window = false;
        self
    }

    fn build_query(&self, org: &OrganizationDetails) -> String {
        let mut lines = Vec::<String>::with_capacity(10);

        // Add any import stanzas
        self.imports.iter().for_each(|i| lines.push(format!("import \"{i}\"")));

        // Add the bucket
        lines.push(format!("from(bucket: \"{}\")", org.influx_bucket));

        // Add a range limit
        lines.push(format!("|> {}", self.period.range()));

        // Add the measurement filter
        if let Some(measurement) = &self.measurement {
            lines.push(format!("|> filter(fn: (r) => r[\"_measurement\"] == \"{}\")", measurement));
        }

        // Add fields filters
        if !self.fields.is_empty() {
            let mut fields = String::new();
            for field in self.fields.iter() {
                if !fields.is_empty() {
                    fields.push_str(" or ");
                }
                fields.push_str(&format!("r[\"_field\"] == \"{}\"", field));
            }
            lines.push(format!("|> filter(fn: (r) => {})", fields));
        }

        // Filter by organization id
        lines.push(format!("|> filter(fn: (r) => r[\"organization_id\"] == \"{}\")", org.key));

        // Filter by host_id
        if let Some(host_id) = &self.host_id {
            lines.push(format!("|> filter(fn: (r) => r[\"host_id\"] == \"{}\")", host_id));
        }

        // Group by
        if !self.group_by.is_empty() {
            let mut group_by = String::new();
            for group in self.group_by.iter() {
                if !group_by.is_empty() {
                    group_by.push_str(", ");
                }
                group_by.push_str(&format!("\"{}\"", group));
            }
            lines.push(format!("|> group(columns: [{}])", group_by));
        }

        // Aggregate Window
        if self.aggregate_window {
            lines.push(format!("|> {}", self.period.aggregate_window()));
        } else {
            lines.push(format!("|> {}", self.period.sample()));
        }

        // Yield as
        if let Some(yield_as) = &self.yield_as {
            lines.push(format!("|> yield(name: \"{}\")", yield_as));
        }

        // Combine
        lines.join("\n")
    }

    #[instrument(skip(self, cnn, key))]
    pub async fn execute<T>(&self, cnn: &Pool<Postgres>, key: &str) -> Result<Vec<T>> 
    where T: FromMap + std::fmt::Debug
    {
        if let Some(org) = get_org_details(cnn, key).await {
            let influx_url = format!("http://{}:8086", org.influx_host);
            let client = Client::new(influx_url, &org.influx_org, &org.influx_token);
            let query_string = self.build_query(&org);
            tracing::info!("{query_string}");
            let query = Query::new(query_string);
            let rows = client.query::<T>(Some(query)).await;
            if let Ok(rows) = rows {
                Ok(rows)
            } else {
                tracing::error!("InfluxDb query error: {rows:?}");
                Err(Error::msg("Influx query error"))
            }
        } else {
            Err(Error::msg("Organization not found"))
        }
    }
}