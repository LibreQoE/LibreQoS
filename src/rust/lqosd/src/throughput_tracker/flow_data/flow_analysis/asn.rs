use std::{io::Read, net::IpAddr, path::Path};
use serde::Deserialize;

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
    
        let mut retries = 0;
        while retries < 3 {
            if file_path.exists() {
                break;
            }
            Self::download()?;
            retries += 1;
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
}
