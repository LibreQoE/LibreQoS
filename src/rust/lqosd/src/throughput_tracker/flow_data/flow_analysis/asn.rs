//! Obtain ASN and geo mappings from IP addresses for flow
//! analysis.


use std::{io::Read, net::IpAddr, path::Path};
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
struct AsnEncoded {
    network: IpAddr,
    prefix: u8,
    pub asn: u32,
    organization: String,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct GeoIpLocation {
    network: IpAddr,
    prefix: u8,
    latitude: f64,
    longitude: f64,
    city_and_country: String,

}

#[derive(Deserialize)]
struct Geobin {
    asn: Vec<AsnEncoded>,
    geo: Vec<GeoIpLocation>,
}

pub struct GeoTable {
    asn_trie: ip_network_table::IpNetworkTable<AsnEncoded>,
    geo_trie: ip_network_table::IpNetworkTable<GeoIpLocation>,
}

impl GeoTable {
    const FILENAME: &'static str = "geo.bin";

    fn file_path() -> std::path::PathBuf {
        Path::new(&lqos_config::load_config().unwrap().lqos_directory)
            .join(Self::FILENAME)
    }

    fn download() -> anyhow::Result<()> {
        log::info!("Downloading ASN-IP Table");
        let file_path = Self::file_path();
        let url = "https://stats.libreqos.io/geo.bin";
        let response = reqwest::blocking::get(url)?;
        let content = response.bytes()?;
        let bytes = &content[0..];
        std::fs::write(file_path, bytes)?;
        Ok(())
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::file_path();
        if !path.exists() {
            log::info!("geo.bin not found - trying to download it");
            Self::download()?;
        }

        // Decompress and deserialize
        let file = std::fs::File::open(path)?;
        let mut buffer = Vec::new();
        flate2::read::GzDecoder::new(file).read_to_end(&mut buffer)?;
        let geobin: Geobin = bincode::deserialize(&buffer)?;

        // Build the ASN trie
        log::info!("Building ASN trie");
        let mut asn_trie = ip_network_table::IpNetworkTable::<AsnEncoded>::new();
        for entry in geobin.asn {
            let (ip, prefix) = match entry.network {
                IpAddr::V4(ip) => (ip.to_ipv6_mapped(), entry.prefix+96 ),
                IpAddr::V6(ip) => (ip, entry.prefix),
            };
            if let Ok(ip) = ip_network::Ipv6Network::new(ip, prefix) {
                asn_trie.insert(ip, entry);
            }
        }

        // Build the GeoIP trie
        log::info!("Building GeoIP trie");
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

        log::info!("GeoTables loaded, {}-{} records.", asn_trie.len().1, geo_trie.len().1);

        Ok(Self {
            asn_trie,
            geo_trie,
        })
    }

    pub fn find_asn(&self, ip: IpAddr) -> Option<u32> {
        log::debug!("Looking up ASN for IP: {:?}", ip);
        let ip = match ip {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };
        if let Some(matched) = self.asn_trie.longest_match(ip) {
            log::debug!("Matched ASN: {:?}", matched.1.asn);
            Some(matched.1.asn)
        } else {
            log::debug!("No ASN found");
            None
        }
    }

    pub fn find_owners_by_ip(&self, ip: IpAddr) -> (String, String) {
        log::debug!("Looking up ASN for IP: {:?}", ip);
        let ip = match ip {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };
        let mut owners = String::new();
        let mut country = String::new();

        if let Some(matched) = self.asn_trie.longest_match(ip) {
            log::debug!("Matched ASN: {:?}", matched.1.asn);
            owners = matched.1.organization.clone();
        }
        if let Some(matched) = self.geo_trie.longest_match(ip) {
            log::debug!("Matched Geo: {:?}", matched.1.city_and_country);
            country = matched.1.city_and_country.clone();
        }

        (owners, country)
    }

    pub fn find_lat_lon_by_ip(&self, ip: IpAddr) -> (f64, f64) {
        log::debug!("Looking up ASN for IP: {:?}", ip);
        let ip = match ip {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };

        if let Some(matched) = self.geo_trie.longest_match(ip) {
            log::debug!("Matched Geo: {:?}", matched.1.city_and_country);
            return (matched.1.latitude, matched.1.longitude);
        }

        (0.0, 0.0)
    }
}

///////////////////////////////////////////////////////////////////////

/*
/// Structure to represent the on-disk structure for files
/// from: https://iptoasn.com/
/// Specifically: https://iptoasn.com/data/ip2asn-combined.tsv.gz
#[derive(Deserialize, Debug, Clone)]
pub struct Ip2AsnRow {
    pub start_ip: IpAddr,
    pub end_ip: IpAddr,
    pub asn: u32,
    pub country: String,
    pub owners: String,
}

pub struct AsnTable {
    asn_table: Vec<Ip2AsnRow>,
}

impl AsnTable {
    pub fn new() -> anyhow::Result<Self> {
        if !Self::exists() {
            Self::download()?;
        }
        let asn_table = Self::build_asn_table()?;
        log::info!("Setup ASN Table with {} entries.", asn_table.len());
        Ok(Self {
            asn_table,
        })
    }

    fn file_path() -> std::path::PathBuf {
        Path::new(&lqos_config::load_config().unwrap().lqos_directory)
            .join("ip2asn-combined.tsv")
    }

    fn download() -> anyhow::Result<()> {
        log::info!("Downloading ASN-IP Table");
        let file_path = Self::file_path();
        let url = "https://iptoasn.com/data/ip2asn-combined.tsv.gz";
        let response = reqwest::blocking::get(url)?;
        let content = response.bytes()?;
        let bytes = &content[0..];
        let mut decompresser = flate2::read::GzDecoder::new(bytes);
        let mut buf = Vec::new();
        decompresser.read_to_end(&mut buf)?;
        std::fs::write(file_path, buf)?;
        Ok(())
    }

    fn exists() -> bool {
        Self::file_path().exists()
    }

    fn build_asn_table() -> anyhow::Result<Vec<Ip2AsnRow>> {
        let file_path = Self::file_path();
    
        if !file_path.exists() {
            let mut retries = 0;
            while retries < 3 {
                if file_path.exists() {
                    break;
                }
                Self::download()?;
                retries += 1;
            }
        }

        if !file_path.exists() {
            anyhow::bail!("IP to ASN file not found: {:?}", file_path);
        }
        let in_file = std::fs::File::open(file_path)?;
    
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(false)
            .delimiter(b'\t')
            .double_quote(false)
            .escape(Some(b'\\'))
            .flexible(true)
            .comment(Some(b'#'))
            .from_reader(in_file);
        
        let mut output = Vec::new();
        for result in rdr.deserialize() {
            let record: Ip2AsnRow = result?;
            output.push(record);
        }
        output.sort_by(|a, b| a.start_ip.cmp(&b.start_ip));
        Ok(output)
    }

    pub fn find_asn(&self, ip: IpAddr) -> Option<Ip2AsnRow> {
        self.asn_table.binary_search_by(|probe| {
            if ip < probe.start_ip {
                std::cmp::Ordering::Greater
            } else if ip > probe.end_ip {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        }).map(|idx| self.asn_table[idx].clone()).ok()
    }

    pub fn find_asn_by_id(&self, asn: u32) -> Option<Ip2AsnRow> {
        self.asn_table.iter().find(|row| row.asn == asn).map(|row| row.clone())
    }
}
*/