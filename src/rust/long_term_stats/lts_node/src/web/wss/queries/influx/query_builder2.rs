use influxdb2::{Client, models::Query};
use influxdb2_structmap::FromMap;
use pgdb::{sqlx::{Pool, Postgres}, OrganizationDetails, organization_cache::get_org_details};
use super::InfluxTimePeriod;

pub struct QueryBuilder<'a> {
    lines: Vec<String>,
    period: Option<&'a InfluxTimePeriod>,
    org: Option<OrganizationDetails>,
}

#[allow(dead_code)]
impl <'a> QueryBuilder <'a> {
    /// Construct a new, completely empty query.
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            period: None,
            org: None,        
        }
    }

    pub fn with_period(mut self, period: &'a InfluxTimePeriod) -> Self {
        self.period = Some(period);
        self
    }

    pub fn with_org(mut self, org: OrganizationDetails) -> Self {
        self.org = Some(org);
        self
    }

    pub async fn derive_org(mut self, cnn: &Pool<Postgres>, key: &str) -> QueryBuilder<'a> {
        let org = get_org_details(cnn, key).await;
        self.org = org;
        self
    }

    pub fn add_line(mut self, line: &str) -> Self {
        self.lines.push(line.to_string());
        self
    }

    pub fn add_lines(mut self, lines: &[&str]) -> Self {
        for line in lines.iter() {
            self.lines.push(line.to_string());
        }
        self
    }

    pub fn bucket(mut self) -> Self {
        if let Some(org) = &self.org {
            self.lines.push(format!("from(bucket: \"{}\")", org.influx_bucket));
        } else {
            tracing::warn!("No organization in query, cannot add bucket");
        }
        self
    }

    pub fn range(mut self) -> Self {
        if let Some(period) = &self.period {
            self.lines.push(format!("|> {}", period.range()));
        } else {
            tracing::warn!("No period in query, cannot add range");
        }
        self
    }

    pub fn filter(mut self, filter: &str) -> Self {
        if !filter.is_empty() {
            self.lines.push(format!("|> filter(fn: (r) => {})", filter));
        }
        self
    }

    pub fn filter_and(mut self, filters: &[&str]) -> Self {
        let all_filters = filters.join(" and ");
        self.lines.push(format!("|> filter(fn: (r) => {})", all_filters));
        self
    }

    pub fn measure_field_org(mut self, measurement: &str, field: &str) -> Self {
        if let Some(org) = &self.org {
            self.lines.push(format!("|> filter(fn: (r) => r[\"_field\"] == \"{}\" and r[\"_measurement\"] == \"{}\" and r[\"organization_id\"] == \"{}\")", field, measurement, org.key));
        } else {
            tracing::warn!("No organization in query, cannot add measure_field_org");
        }
        self
    }

    pub fn aggregate_window(mut self) -> Self {
        if let Some(period) = &self.period {
            self.lines.push(format!("|> {}", period.aggregate_window()));
        } else {
            tracing::warn!("No period in query, cannot add aggregate_window");
        }
        self
    }

    pub fn group(mut self, columns: &[&str]) -> Self {
        let group_by = columns.join(", ");
        self.lines.push(format!("|> group(columns: [\"{}\"])", group_by));
        self
    }

    pub async fn execute<T>(&self) -> anyhow::Result<Vec<T>>
    where T: FromMap + std::fmt::Debug 
    {
        let qs = self.lines.join("\n");
        tracing::info!("Query:\n{}", qs);
        if let Some(org) = &self.org {
            let influx_url = format!("http://{}:8086", org.influx_host);
            let client = Client::new(influx_url, &org.influx_org, &org.influx_token);
            let query = Query::new(qs.clone());
            let rows = client.query::<T>(Some(query)).await;
            if let Ok(rows) = rows {
                Ok(rows)
            } else {
                tracing::error!("InfluxDb query error: {rows:?} for: {qs}");
                anyhow::bail!("Influx query error");
            }
        } else {
            anyhow::bail!("No organization in query, cannot execute");
        }
    }
}