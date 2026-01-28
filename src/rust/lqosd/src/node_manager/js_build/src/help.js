import {displayDaveMemorial} from "./dave";
import {get_ws_client} from "./pubsub/ws";

function escapeHtml(str) {
    if (str === null || str === undefined) {
        return "";
    }
    return String(str)
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;")
        .replaceAll("\"", "&quot;")
        .replaceAll("'", "&#039;");
}

function getCommentor() {
    const el = document.getElementById("username");
    const name = el ? (el.textContent || "").trim() : "";
    return name || "Anonymous";
}

function statusBadge(status) {
    const text = (status || "").toUpperCase();
    if (text === "CLOSED") {
        return { cls: "text-bg-secondary", label: "CLOSED" };
    }
    if (text === "OPEN") {
        return { cls: "text-bg-warning", label: "OPEN" };
    }
    return { cls: "text-bg-primary", label: "NEW" };
}

function fmtUnix(ts) {
    if (!ts) {
        return "";
    }
    try {
        return new Date(ts * 1000).toLocaleString();
    } catch {
        return "";
    }
}

let wsClient = null;
let currentTicketId = null;

function setAlert(id, message) {
    const el = document.getElementById(id);
    if (!el) return;
    if (!message) {
        el.classList.add("d-none");
        el.textContent = "";
        return;
    }
    el.classList.remove("d-none");
    el.textContent = message;
}

function renderTicketList(tickets) {
    const list = document.getElementById("ticketList");
    if (!list) return;
    if (!tickets || tickets.length === 0) {
        list.innerHTML = `<div class="text-body-secondary small px-2 py-3">
            <i class="fa fa-inbox me-1"></i> No tickets yet.
        </div>`;
        return;
    }

    list.innerHTML = tickets
        .map((t) => {
            const badge = statusBadge(t.status);
            const subject = escapeHtml(t.subject || "(no subject)");
            const updated = fmtUnix(t.updated_at);
            return `<button type="button" class="list-group-item list-group-item-action d-flex justify-content-between align-items-start gap-2"
                    data-ticket-id="${t.id}">
                <div class="me-auto">
                    <div class="fw-semibold">${subject}</div>
                    <div class="text-body-secondary small">Updated ${escapeHtml(updated)}</div>
                </div>
                <div class="d-flex flex-column align-items-end gap-1">
                    <span class="badge ${badge.cls}">${badge.label}</span>
                    <span class="badge text-bg-light border">P${t.priority ?? 0}</span>
                </div>
            </button>`;
        })
        .join("");

    // Wire click handlers
    list.querySelectorAll("[data-ticket-id]").forEach((btn) => {
        btn.addEventListener("click", () => {
            const id = parseInt(btn.getAttribute("data-ticket-id"), 10);
            if (!id || Number.isNaN(id)) return;
            openTicket(id);
        });
    });
}

function setTicketViewLoading() {
    setAlert("ticketViewAlert", "");
    $("#ticketViewSubject").text("Loading…");
    $("#ticketViewMeta").text("");
    $("#ticketViewBody").text("");
    $("#ticketViewComments").html(`<div class="text-body-secondary small"><i class="fa fa-spinner fa-spin me-1"></i> Loading…</div>`);
    $("#ticketViewStatus").removeClass().addClass("badge text-bg-secondary").text("…");
    $("#ticketViewPriority").text("");
}

function openTicket(ticketId) {
    currentTicketId = ticketId;
    setTicketViewLoading();
    const modalEl = document.getElementById("ticketViewModal");
    if (modalEl) {
        const modal = bootstrap.Modal.getOrCreateInstance(modalEl);
        modal.show();
    }
    wsClient.send({ SupportTicketGet: { ticket_id: ticketId } });
}

function renderTicketView(ticket) {
    if (!ticket) {
        setAlert("ticketViewAlert", "Ticket not found.");
        return;
    }

    $("#ticketViewSubject").text(ticket.subject || "");
    const meta = [`#${ticket.id}`];
    if (ticket.updated_at) meta.push(`Updated ${fmtUnix(ticket.updated_at)}`);
    $("#ticketViewMeta").text(meta.join(" • "));

    $("#ticketViewBody").text(ticket.body || "");

    const badge = statusBadge(ticket.status);
    $("#ticketViewStatus").removeClass().addClass(`badge ${badge.cls}`).text(badge.label);
    $("#ticketViewPriority").text(`P${ticket.priority ?? 0}`);

    const comments = ticket.comments || [];
    if (!comments.length) {
        $("#ticketViewComments").html(`<div class="text-body-secondary small"><i class="fa fa-comment-dots me-1"></i> No comments yet.</div>`);
        return;
    }
    const html = comments
        .map((c) => {
            const who = escapeHtml(c.commentor || "Unknown");
            const when = escapeHtml(fmtUnix(c.date));
            const body = escapeHtml(c.body || "");
            return `<div class="border rounded p-2">
                <div class="d-flex justify-content-between flex-wrap gap-2 mb-1">
                    <div class="fw-semibold small">${who}</div>
                    <div class="text-body-secondary small">${when}</div>
                </div>
                <div class="small" style="white-space: pre-wrap;">${body}</div>
            </div>`;
        })
        .join("");
    $("#ticketViewComments").html(html);
}

function refreshTickets() {
    $("#ticketList").html(`<div class="text-body-secondary small px-2 py-3">
        <i class="fa fa-spinner fa-spin me-1"></i> Loading tickets…
    </div>`);
    wsClient.send({ SupportTicketList: {} });
}

function openNewTicketModal() {
    setAlert("newTicketAlert", "");
    $("#newTicketSubject").val("");
    $("#newTicketPriority").val("3");
    $("#newTicketBody").val("");
    const modalEl = document.getElementById("newTicketModal");
    if (modalEl) {
        const modal = bootstrap.Modal.getOrCreateInstance(modalEl);
        modal.show();
    }
}

function submitNewTicket() {
    setAlert("newTicketAlert", "");
    const subject = String($("#newTicketSubject").val() || "").trim();
    const body = String($("#newTicketBody").val() || "").trim();
    const priority = parseInt(String($("#newTicketPriority").val() || "3"), 10);
    if (!subject) {
        setAlert("newTicketAlert", "Subject is required.");
        return;
    }
    if (!body) {
        setAlert("newTicketAlert", "Details are required.");
        return;
    }
    if (Number.isNaN(priority) || priority < 0 || priority > 5) {
        setAlert("newTicketAlert", "Priority must be between 0 and 5.");
        return;
    }
    wsClient.send({
        SupportTicketCreate: {
            subject,
            priority,
            body,
            commentor: getCommentor(),
        },
    });
}

function submitComment() {
    setAlert("ticketViewAlert", "");
    const body = String($("#ticketCommentBody").val() || "").trim();
    if (!currentTicketId) {
        setAlert("ticketViewAlert", "No ticket selected.");
        return;
    }
    if (!body) {
        setAlert("ticketViewAlert", "Comment cannot be empty.");
        return;
    }
    $("#ticketCommentBody").val("");
    wsClient.send({
        SupportTicketAddComment: {
            ticket_id: currentTicketId,
            commentor: getCommentor(),
            body,
        },
    });
}

function initSupportTickets() {
    const hasInsight = typeof window.hasInsight !== "undefined" ? !!window.hasInsight : false;
    const noInsight = document.getElementById("ticketGateNoInsight");
    const yesInsight = document.getElementById("ticketGateInsight");
    const btnNew = document.getElementById("btnNewTicket");
    const btnRefresh = document.getElementById("btnRefreshTickets");

    if (!hasInsight) {
        if (noInsight) noInsight.classList.remove("d-none");
        if (yesInsight) yesInsight.classList.add("d-none");
        if (btnNew) btnNew.disabled = true;
        if (btnRefresh) btnRefresh.disabled = true;
        renderTicketList([]);
        return;
    }

    if (noInsight) noInsight.classList.add("d-none");
    if (yesInsight) yesInsight.classList.remove("d-none");
    if (btnNew) btnNew.disabled = false;
    if (btnRefresh) btnRefresh.disabled = false;

    wsClient = get_ws_client();
    wsClient.on("SupportTicketListResult", (msg) => {
        renderTicketList(msg.tickets || []);
    });
    wsClient.on("SupportTicketGetResult", (msg) => {
        if (msg.ticket && currentTicketId && msg.ticket.id !== currentTicketId) {
            return;
        }
        renderTicketView(msg.ticket);
    });
    wsClient.on("SupportTicketCreateResult", (msg) => {
        // Close modal and refresh list
        const modalEl = document.getElementById("newTicketModal");
        if (modalEl) {
            bootstrap.Modal.getOrCreateInstance(modalEl).hide();
        }
        refreshTickets();
        if (msg.ticket && msg.ticket.id) {
            openTicket(msg.ticket.id);
        }
    });
    wsClient.on("SupportTicketAddCommentResult", (msg) => {
        if (!msg.ok) {
            setAlert("ticketViewAlert", "Failed to add comment.");
            return;
        }
        if (currentTicketId) {
            wsClient.send({ SupportTicketGet: { ticket_id: currentTicketId } });
        }
    });
    wsClient.on("Error", (msg) => {
        const message = msg && msg.message ? String(msg.message) : "Unknown error";
        setAlert("ticketViewAlert", message);
        setAlert("newTicketAlert", message);
        // Also surface in list area if empty/loading
        const list = document.getElementById("ticketList");
        if (list && (!list.children || list.children.length === 0)) {
            list.innerHTML = `<div class="text-danger small px-2 py-3">
                <i class="fa fa-triangle-exclamation me-1"></i> ${escapeHtml(message)}
            </div>`;
        }
    });

    refreshTickets();
}

// Perform wireups
$("#btnDave").click(displayDaveMemorial);
$("#btnNewTicket").on("click", openNewTicketModal);
$("#btnSubmitTicket").on("click", submitNewTicket);
$("#btnRefreshTickets").on("click", () => refreshTickets());
$("#btnSubmitComment").on("click", submitComment);

initSupportTickets();
