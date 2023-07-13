use sha2::Sha256;
use sha2::Digest;

pub(crate) fn hash_password(password: &str) -> String {
    let salted = format!("!x{password}_SaltIsGoodForYou");
    let mut sha256 = Sha256::new();
    sha256.update(salted);
    format!("{:X}", sha256.finalize())
  }