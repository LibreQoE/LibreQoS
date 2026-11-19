import {formatDateTime} from "./i18n/format.js";
import {t} from "./i18n/index.js";
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
    return name || t("help.comments.anonymous");
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

// The comment timestamps and the list's "Updated" line read the shared Intl
// formatters: a page that builds a clock by hand renders an English date in
// every locale, and the contract test rejects the call.
function fmtUnix(ts) {
    if (!ts) {
        return "";
    }
    try {
        return formatDateTime(new Date(ts * 1000));
    } catch {
        return "";
    }
}

/** The "Updated …" line around a formatted timestamp; empty when there is none. */
function updateLine(time) {
    return time ? t("common.state.updated", {time}) : "";
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
            <i class="fa fa-inbox me-1"></i> ${escapeHtml(t("help.tickets.empty"))}
        </div>`;
        return;
    }

    list.innerHTML = tickets
        .map((ticket) => {
            const badge = statusBadge(ticket.status);
            const subject = escapeHtml(ticket.subject || t("help.tickets.no_subject"));
            const updated = fmtUnix(ticket.updated_at);
            return `<button type="button" class="list-group-item list-group-item-action d-flex justify-content-between align-items-start gap-2"
                    data-ticket-id="${ticket.id}">
                <div class="me-auto">
                    <div class="fw-semibold">${subject}</div>
                    <div class="text-body-secondary small">${escapeHtml(updateLine(updated))}</div>
                </div>
                <div class="d-flex flex-column align-items-end gap-1">
                    <span class="badge ${badge.cls}">${badge.label}</span>
                    <span class="badge text-bg-light border">P${ticket.priority ?? 0}</span>
                </div>
            </button>`;
        })
        .join("");

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
    $("#ticketViewSubject").text(t("common.state.loading"));
    $("#ticketViewMeta").text("");
    $("#ticketViewBody").text("");
    $("#ticketViewComments").html(`<div class="text-body-secondary small"><i class="fa fa-spinner fa-spin me-1"></i> ${escapeHtml(t("common.state.loading"))}</div>`);
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
        setAlert("ticketViewAlert", t("help.ticket.not_found"));
        return;
    }

    $("#ticketViewSubject").text(ticket.subject || "");
    const meta = [`#${ticket.id}`];
    if (ticket.updated_at) meta.push(updateLine(fmtUnix(ticket.updated_at)));
    $("#ticketViewMeta").text(meta.join(" • "));

    $("#ticketViewBody").text(ticket.body || "");

    const badge = statusBadge(ticket.status);
    $("#ticketViewStatus").removeClass().addClass(`badge ${badge.cls}`).text(badge.label);
    $("#ticketViewPriority").text(`P${ticket.priority ?? 0}`);

    const comments = ticket.comments || [];
    if (!comments.length) {
        $("#ticketViewComments").html(`<div class="text-body-secondary small"><i class="fa fa-comment-dots me-1"></i> ${escapeHtml(t("help.comments.empty"))}</div>`);
        return;
    }
    const html = comments
        .map((c) => {
            const who = escapeHtml(c.commentor || t("help.comments.unknown_author"));
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
        <i class="fa fa-spinner fa-spin me-1"></i> ${escapeHtml(t("help.tickets.loading"))}
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
        setAlert("newTicketAlert", t("help.ticket.subject_required"));
        return;
    }
    if (!body) {
        setAlert("newTicketAlert", t("help.ticket.details_required"));
        return;
    }
    if (Number.isNaN(priority) || priority < 0 || priority > 5) {
        setAlert("newTicketAlert", t("help.ticket.priority_invalid"));
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
        setAlert("ticketViewAlert", t("help.ticket.none_selected"));
        return;
    }
    if (!body) {
        setAlert("ticketViewAlert", t("help.comments.empty_error"));
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
    const hasSupportTickets =
        typeof window.hasSupportTickets !== "undefined"
            ? !!window.hasSupportTickets
            : false;
    const liveControlAvailable =
        typeof window.liveControlAvailable !== "undefined"
            ? !!window.liveControlAvailable
            : false;
    const noInsight = document.getElementById("ticketGateNoInsight");
    const yesInsight = document.getElementById("ticketGateInsight");
    const btnNew = document.getElementById("btnNewTicket");
    const btnRefresh = document.getElementById("btnRefreshTickets");

    if (!hasSupportTickets) {
        if (noInsight) noInsight.classList.remove("d-none");
        if (yesInsight) yesInsight.classList.add("d-none");
        if (btnNew) btnNew.disabled = true;
        if (btnRefresh) btnRefresh.disabled = true;
        renderTicketList([]);
        return;
    }

    if (noInsight) noInsight.classList.add("d-none");
    if (yesInsight) yesInsight.classList.remove("d-none");
    if (btnNew) btnNew.disabled = !liveControlAvailable;
    if (btnRefresh) btnRefresh.disabled = !liveControlAvailable;

    if (!liveControlAvailable) {
        setAlert("ticketListAlert", t("config.lts.alert.control_unavailable"));
    }

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
            setAlert("ticketViewAlert", t("help.comments.add_failed"));
            return;
        }
        if (currentTicketId) {
            wsClient.send({ SupportTicketGet: { ticket_id: currentTicketId } });
        }
    });
    // translatable: false — "Error" is the websocket event name this handler binds to, not a label
    wsClient.on("Error", (msg) => {
        const message = msg && msg.message ? String(msg.message) : t("help.errors.unknown");
        if (
            // The server sends this reason as English text; the comparison is a
            // protocol check, not copy, so the literal stays out of the catalog.
            // translatable: false — "Support tickets require an entitled license." is the server reason matched here to reveal the licence gate, never rendered from this literal
            message.includes("Support tickets require an entitled license.") ||
            // translatable: false — "license valid, control service unavailable" is the server reason matched here to reveal the licence gate, never rendered from this literal
            message.includes("license valid, control service unavailable")
        ) {
            if (noInsight) noInsight.classList.remove("d-none");
            if (yesInsight) yesInsight.classList.add("d-none");
            if (btnNew) btnNew.disabled = true;
            if (btnRefresh) btnRefresh.disabled = true;
        }
        setAlert("ticketViewAlert", message);
        setAlert("newTicketAlert", message);
        const list = document.getElementById("ticketList");
        if (list) {
            list.innerHTML = `<div class="text-danger small px-2 py-3">
                <i class="fa fa-triangle-exclamation me-1"></i> ${escapeHtml(message)}
            </div>`;
        }
    });

    refreshTickets();
}

// Perform wireups
$("#btnNewTicket").on("click", openNewTicketModal);
$("#btnSubmitTicket").on("click", submitNewTicket);
$("#btnRefreshTickets").on("click", () => refreshTickets());
$("#btnSubmitComment").on("click", submitComment);

initSupportTickets();
