use std::time::Duration;
use axum::extract::Path;
use axum::Json;
use lqos_bus::{bus_request, AsnCountryListEntry, AsnListEntry, AsnProtocolListEntry, BusRequest, BusResponse, FlowTimeline};
use lqos_utils::unix_time::{time_since_boot, unix_now};
use crate::shaped_devices_tracker::SHAPED_DEVICES;

pub async fn asn_list() -> Json<Vec<AsnListEntry>> {
    let Ok(replies) = bus_request(vec![BusRequest::FlowAsnList]).await else {
        return Json(Vec::new());
    };

    for reply in replies {
        if let BusResponse::FlowAsnList(asn_list) = reply {
            return Json(asn_list);
        }
    }

    Json(Vec::new())
}

pub async fn country_list() -> Json<Vec<AsnCountryListEntry>> {
    let Ok(replies) = bus_request(vec![BusRequest::FlowCountryList]).await else {
        return Json(Vec::new());
    };
    for reply in replies {
        if let BusResponse::FlowCountryList(country_list) = reply {
            return Json(country_list);
        }
    }
    Json(Vec::new())
}

pub async fn protocol_list() -> Json<Vec<AsnProtocolListEntry>> {
    let Ok(replies) = bus_request(vec![BusRequest::FlowProtocolList]).await else {
        return Json(Vec::new());
    };
    for reply in replies {
        if let BusResponse::FlowProtocolList(protocol_list) = reply {
            return Json(protocol_list);
        }
    }
    Json(Vec::new())
}

pub async fn flow_timeline(Path(asn_id): Path<u32>) -> Json<Vec<FlowTimeline>> {
    let Ok(replies) = bus_request(vec![BusRequest::FlowTimeline(asn_id)]).await else {
        return Json(Vec::new());
    };
    for reply in replies {
        if let BusResponse::FlowTimeline(flows) = reply {
            return Json(flows);
        }
    }
    Json(Vec::new())
}

pub async fn country_timeline(Path(iso_code): Path<String>) -> Json<Vec<FlowTimeline>> {
    let Ok(replies) = bus_request(vec![BusRequest::FlowCountryTimeline(iso_code)]).await else {
        return Json(Vec::new());
    };
    for reply in replies {
        if let BusResponse::FlowCountryTimeline(flows) = reply {
            return Json(flows);
        }
    }
    Json(Vec::new())
}

pub async fn protocol_timeline(Path(protocol_name): Path<String>) -> Json<Vec<FlowTimeline>> {
    let Ok(replies) = bus_request(vec![BusRequest::FlowProtocolTimeline(protocol_name)]).await else {
        return Json(Vec::new());
    };
    for reply in replies {
        if let BusResponse::FlowProtocolTimeline(flows) = reply {
            return Json(flows);
        }
    }
    Json(Vec::new())
}