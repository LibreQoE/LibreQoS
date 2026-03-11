import {listenExecutiveHeatmaps, averageWithCount, renderTable, renderCircuitLink} from "./executive_utils";

const THRESHOLD = 80;
const MIN_SAMPLES = 10;

function buildRows(data) {
    const rows = (data.circuits || []).map(circuit => {
        const down = averageWithCount(circuit.blocks?.download || []);
        const up = averageWithCount(circuit.blocks?.upload || []);
        return {
            name: circuit.circuit_name || circuit.circuit_id || `Circuit ${circuit.circuit_hash}`,
            circuit_id: circuit.circuit_id,
            down,
            up,
        };
    }).filter(row =>
        row.down.count >= MIN_SAMPLES &&
        row.up.count >= MIN_SAMPLES &&
        (row.down.avg || 0) >= THRESHOLD &&
        (row.up.avg || 0) >= THRESHOLD
    );

    rows.sort((a, b) => ((b.down.avg || 0) + (b.up.avg || 0)) - ((a.down.avg || 0) + (a.up.avg || 0)));
    return rows;
}

function render(data) {
    const rows = buildRows(data);
    renderTable("executiveCircuitsDueTable", [
        { header: "Circuit", render: (r) => renderCircuitLink(r.name, r.circuit_id) },
        { header: "Avg Down Util (%)", render: (r) => r.down.avg !== null ? r.down.avg.toFixed(1) : "—" },
        { header: "Avg Up Util (%)", render: (r) => r.up.avg !== null ? r.up.avg.toFixed(1) : "—" },
    ], rows, "No circuits meet the 80%+ utilization threshold yet.");
}

listenExecutiveHeatmaps(render);
