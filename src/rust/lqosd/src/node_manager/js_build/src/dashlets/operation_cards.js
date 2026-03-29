import {mkBadge} from "./bakery_shared";

export function renderOperationCards(host, groups, options = {}) {
    const emptyText = options.emptyText || "No recent operations";
    host.innerHTML = "";

    if (!Array.isArray(groups) || groups.length === 0) {
        const empty = document.createElement("div");
        empty.classList.add("border", "rounded", "p-2", "text-muted", "small");
        empty.textContent = emptyText;
        host.appendChild(empty);
        return;
    }

    groups.forEach((group) => {
        const card = document.createElement("div");
        card.classList.add("border", "rounded", "p-2", "bg-body-tertiary");

        const top = document.createElement("div");
        top.classList.add("d-flex", "justify-content-between", "align-items-start", "gap-2", "flex-wrap", "mb-2");

        const titleWrap = document.createElement("div");
        const title = document.createElement("div");
        title.classList.add("fw-semibold");
        title.textContent = group.label || "Operation";
        titleWrap.appendChild(title);

        const right = document.createElement("div");
        right.classList.add("d-flex", "flex-wrap", "gap-2", "align-items-center");
        right.appendChild(
            mkBadge(
                group.outcomeLabel || "Info",
                group.outcomeClass || "bg-light text-secondary border",
                group.outcomeTitle || "",
            ),
        );

        top.appendChild(titleWrap);
        top.appendChild(right);
        card.appendChild(top);

        if ((group.summary || "").toString().trim()) {
            const summary = document.createElement("div");
            summary.classList.add("small");
            summary.textContent = group.summary;
            if (group.summaryTitle) {
                summary.title = group.summaryTitle;
            }
            card.appendChild(summary);
        }

        const footer = document.createElement("div");
        footer.classList.add("d-flex", "justify-content-between", "align-items-center", "gap-2", "mt-2", "small", "text-body-secondary", "flex-wrap");

        const footerLeft = document.createElement("div");
        footerLeft.textContent = group.footerLeft || "";
        footer.appendChild(footerLeft);

        const footerRight = document.createElement("div");
        footerRight.textContent = group.footerRight || "";
        footer.appendChild(footerRight);
        card.appendChild(footer);

        if (Array.isArray(group.stages) && group.stages.length > 0) {
            const progress = document.createElement("div");
            progress.classList.add("progress", "mt-2", "mb-2");
            progress.style.height = "0.45rem";

            const progressBar = document.createElement("div");
            progressBar.classList.add("progress-bar", group.progressBarClass || "bg-info");
            progressBar.style.width = `${group.progressPercent || 0}%`;
            progressBar.setAttribute("role", "progressbar");
            progressBar.setAttribute("aria-valuemin", "0");
            progressBar.setAttribute("aria-valuemax", "100");
            progressBar.setAttribute("aria-valuenow", `${Math.round(group.progressPercent || 0)}`);
            progress.appendChild(progressBar);
            card.appendChild(progress);

            const stages = document.createElement("div");
            stages.classList.add("small", "text-body-secondary");
            stages.textContent = group.stages.join(" -> ");
            card.appendChild(stages);
        }

        host.appendChild(card);
    });
}
