//! Obtain ASN and geo mappings from IP addresses for flow
//! analysis.


use std::{io::Read, net::IpAddr, path::Path};
use fxhash::FxHashMap;
use serde::Deserialize;
use tracing::{debug, info};

#[derive(Deserialize, Clone, Debug)]
struct AsnEncoded {
    network: IpAddr,
    prefix: u8,
    pub asn: u32,
    organization: String,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct GeoIpLocation {
    pub network: IpAddr,
    pub prefix: u8,
    pub latitude: f64,
    pub longitude: f64,
    pub city: String,
    pub country: String,
    pub country_iso_code: String,
}

impl GeoIpLocation {
    pub fn city_and_country(&self) -> String {
        format!("{}, {}", self.city, self.country)
            .trim_end_matches(',')
            .trim()
            .to_string()
    }

}

#[derive(Deserialize)]
struct Geobin {
    asn: Vec<AsnEncoded>,
    geo: Vec<GeoIpLocation>,
}

pub struct GeoTable {
    asn_trie: ip_network_table::IpNetworkTable<AsnEncoded>,
    geo_trie: ip_network_table::IpNetworkTable<GeoIpLocation>,
    asn_lookup: FxHashMap<u32, String>,
}

impl GeoTable {
    const FILENAME: &'static str = "geo2.bin";

    fn file_path() -> std::path::PathBuf {
        Path::new(&lqos_config::load_config().unwrap().lqos_directory)
            .join(Self::FILENAME)
    }

    fn download() -> anyhow::Result<()> {
        debug!("Downloading ASN-IP Table");
        let file_path = Self::file_path();
        let url = "https://stats.libreqos.io/geo2.bin";
        let response = reqwest::blocking::get(url)?;
        let content = response.bytes()?;
        let bytes = &content[0..];
        std::fs::write(file_path, bytes)?;
        Ok(())
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::file_path();
        if !path.exists() {
            info!("geo.bin not found - trying to download it");
            Self::download()?;
        }

        // Decompress and deserialize
        let file = std::fs::File::open(path)?;
        let mut buffer = Vec::new();
        flate2::read::GzDecoder::new(file).read_to_end(&mut buffer)?;
        let geobin: Geobin = bincode::deserialize(&buffer)?;

        // Build the ASN trie and ASN lookup map
        let mut asn_lookup = FxHashMap::default();

        debug!("Building ASN trie");
        let mut asn_trie = ip_network_table::IpNetworkTable::<AsnEncoded>::new();
        for entry in geobin.asn {
            asn_lookup.insert(entry.asn, entry.organization.clone());
            let (ip, prefix) = match entry.network {
                IpAddr::V4(ip) => (ip.to_ipv6_mapped(), entry.prefix+96 ),
                IpAddr::V6(ip) => (ip, entry.prefix),
            };
            if let Ok(ip) = ip_network::Ipv6Network::new(ip, prefix) {
                asn_trie.insert(ip, entry);
            }
        }

        // Build the GeoIP trie
        debug!("Building GeoIP trie");
        let mut geo_trie = ip_network_table::IpNetworkTable::<GeoIpLocation>::new();
        for entry in geobin.geo {
            let (ip, prefix) = match entry.network {
                IpAddr::V4(ip) => (ip.to_ipv6_mapped(), entry.prefix+96 ),
                IpAddr::V6(ip) => (ip, entry.prefix),
            };
            if let Ok(ip) = ip_network::Ipv6Network::new(ip, prefix) {
                geo_trie.insert(ip, entry);
            }
        }

        debug!("GeoTables loaded, {}-{} records.", asn_trie.len().1, geo_trie.len().1);

        Ok(Self {
            asn_trie,
            geo_trie,
            asn_lookup,
        })
    }

    pub fn find_asn(&self, ip: IpAddr) -> Option<u32> {
        debug!("Looking up ASN for IP: {:?}", ip);
        let ip = match ip {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };
        if let Some(matched) = self.asn_trie.longest_match(ip) {
            debug!("Matched ASN: {:?}", matched.1.asn);
            Some(matched.1.asn)
        } else {
            debug!("No ASN found");
            None
        }
    }

    pub fn find_owners_by_ip(&self, ip: IpAddr) -> AsnNameCountryFlag{
        debug!("Looking up ASN for IP: {:?}", ip);
        let ip = match ip {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };
        let mut owners = "Unknown".to_string();
        let mut country = "Unknown".to_string();
        let mut flag = "Unknown".to_string();

        if let Some(matched) = self.asn_trie.longest_match(ip) {
            debug!("Matched ASN: {:?}", matched.1.asn);
            owners = matched.1.organization.clone();
        }
        if let Some(matched) = self.geo_trie.longest_match(ip) {
            debug!("Matched Geo: {:?}", matched.1.city_and_country());
            country = matched.1.city_and_country();
            flag = matched.1.country_iso_code.clone();
        }

        AsnNameCountryFlag {
            name: owners,
            country,
            flag,
        }
    }

    pub fn find_lat_lon_by_ip(&self, ip: IpAddr) -> (f64, f64) {
        debug!("Looking up ASN for IP: {:?}", ip);
        let ip = match ip {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };

        if let Some(matched) = self.geo_trie.longest_match(ip) {
            debug!("Matched Geo: {:?}", matched.1.city_and_country());
            return (matched.1.latitude, matched.1.longitude);
        }

        (0.0, 0.0)
    }

    pub fn find_name_by_id(&self, id: u32) -> String {
        self.asn_lookup.get(&id).cloned().unwrap_or_else(|| "Unknown".to_string())
    }
}

#[derive(Default)]
pub struct AsnNameCountryFlag {
    pub name: String,
    pub country: String,
    pub flag: String,
}