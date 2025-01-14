import {clearDiv} from "../helpers/builders";

class TimeControls {
    constructor(parentId) {
        this.parentId = parentId;
        const periods = ["Live", "1h", "6h", "12h", "24h", "7d"];
        this.activePeriod = periods[0];
        let parent = document.getElementById(parentId);
        clearDiv(parent);
        periods.forEach((period) => {
            let button = document.createElement("button");
            button.id = "tp_" + period;
            button.innerText = period;
            if (period === this.activePeriod) {
                button.classList.add("btn-primary");
            } else {
                button.classList.add("btn-outline-primary");
            }
            button.classList.add("btn", "btn-sm", "me-1");
            button.onclick = () => {
                if (period !== "Live" && !window.hasLts) {
                    createAndShowModal('Feature not available', 'Displaying extended time periods requires an Insight subscription or free trial. Click the "Insight" button in the menu to learn more.');
                    return;
                }

                this.activePeriod = period;
                periods.forEach((p) => {
                    let b = document.getElementById("tp_" + p);
                    if (p === period) {
                        b.classList.remove("btn-outline-primary");
                        b.classList.add("btn-primary");
                    } else {
                        b.classList.remove("btn-primary");
                        b.classList.add("btn-outline-primary");
                    }
                });
                if (window.timeGraphs !== undefined) {
                    window.timeGraphs.forEach((graph) => {
                        if (graph !== null) graph.onTimeChange();
                    });
                }
            };
            parent.appendChild(button);
        });
    }
}

export function showTimeControls(parentId) {
    window.timePeriods = new TimeControls(parentId);
}

function createAndShowModal(title, message) {
    // Create the modal wrapper
    const modal = document.createElement('div');
    modal.className = 'modal fade';
    modal.id = 'dynamicModal';
    modal.tabIndex = -1;
    modal.setAttribute('role', 'dialog');
    modal.setAttribute('aria-labelledby', 'dynamicModalLabel');
    modal.setAttribute('aria-hidden', 'true');

    // Create modal dialog
    const modalDialog = document.createElement('div');
    modalDialog.className = 'modal-dialog';
    modalDialog.setAttribute('role', 'document');

    // Create modal content
    const modalContent = document.createElement('div');
    modalContent.className = 'modal-content';

    // Create modal header
    const modalHeader = document.createElement('div');
    modalHeader.className = 'modal-header';

    const modalTitle = document.createElement('h5');
    modalTitle.className = 'modal-title';
    modalTitle.id = 'dynamicModalLabel';
    modalTitle.textContent = title;

    const closeButton = document.createElement('button');
    closeButton.type = 'button';
    closeButton.className = 'btn-close';
    closeButton.setAttribute('data-bs-dismiss', 'modal');
    closeButton.setAttribute('aria-label', 'Close');

    modalHeader.appendChild(modalTitle);
    modalHeader.appendChild(closeButton);

    // Create modal body
    const modalBody = document.createElement('div');
    modalBody.className = 'modal-body';
    modalBody.textContent = message;

    // Create modal footer
    const modalFooter = document.createElement('div');
    modalFooter.className = 'modal-footer';

    const footerCloseButton = document.createElement('button');
    footerCloseButton.type = 'button';
    footerCloseButton.className = 'btn btn-secondary';
    footerCloseButton.setAttribute('data-bs-dismiss', 'modal');
    footerCloseButton.textContent = 'Close';

    modalFooter.appendChild(footerCloseButton);

    // Assemble modal components
    modalContent.appendChild(modalHeader);
    modalContent.appendChild(modalBody);
    modalContent.appendChild(modalFooter);

    modalDialog.appendChild(modalContent);
    modal.appendChild(modalDialog);

    // Append modal to the body
    document.body.appendChild(modal);

    // Show the modal using Bootstrap's JavaScript API
    const bootstrapModal = new bootstrap.Modal(modal);
    bootstrapModal.show();

    // Remove modal from DOM after it's hidden
    modal.addEventListener('hidden.bs.modal', () => {
        modal.remove();
    });
}
