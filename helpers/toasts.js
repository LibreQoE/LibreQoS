export function createBootstrapToast(toastId, headerSpan, toastBody) {
    // Create the toast div
    const toast = document.createElement('div');
    toast.id = toastId;
    toast.className = 'toast hide'; // Hidden initially
    toast.setAttribute('role', 'alert');
    toast.setAttribute('aria-live', 'assertive');
    toast.setAttribute('aria-atomic', 'true');

    // Create the toast-header div
    const toastHeader = document.createElement('div');
    toastHeader.className = 'toast-header';

    // Create the close button
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'btn-close';
    button.setAttribute('data-bs-dismiss', 'toast');
    button.setAttribute('aria-label', 'Close');

    // Append children to toast-header
    toastHeader.appendChild(headerSpan);
    toastHeader.appendChild(button);

    // Create the toast-body div
    toastBody.className = 'toast-body';

    // Append header and body to toast
    toast.appendChild(toastHeader);
    toast.appendChild(toastBody);

    // Append toast to the toast container
    const target = document.getElementById('toastHolder');
    if (target !== null) {
        target.appendChild(toast);
    } else {
        document.body.appendChild(toast);
    }

    // Fire it up
    const toastJs = new bootstrap.Toast(toast);
    toastJs.show();
}
