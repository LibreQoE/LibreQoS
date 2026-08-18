from typing import Any


def normalize_mac(mac: Any) -> str:
    if mac is None:
        return ""
    value = str(mac).strip()
    if not value:
        return ""
    return value.replace(":", "").replace("-", "").upper()
