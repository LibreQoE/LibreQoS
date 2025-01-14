export function createBootstrapToast(toastId, headerSpan, toastBody) {
    // Create main wrapper div
    const wrapper = document.createElement('div');
    wrapper.className = 'position-fixed bottom-0 end-0 p-3';
    wrapper.style.zIndex = '11';

    // Create the toast div
    const toast = document.createElement('div');
    toast.id = toastId;
    toast.className = 'toast hide';
    toast.setAttribute('role', 'alert');
    toast.setAttribute('aria-live', 'assertive');
    toast.setAttribute('aria-atomic', 'true');

    // Create the toast-header div
    const toastHeader = document.createElement('div');
    toastHeader.className = 'toast-header';

    // Create the image element
    const img = document.createElement('img');
    img.src = '...'; // Replace with the actual image URL
    img.className = 'rounded me-2';
    img.alt = '...';

    // Create the strong element
    const strong = document.createElement('strong');
    strong.className = 'me-auto';
    strong.textContent = 'Bootstrap';

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

    // Append toast to wrapper
    wrapper.appendChild(toast);

    // Append wrapper to body (or another container)
    document.body.appendChild(wrapper);

    // Fire it up
    const toastElement = document.getElementById(toastId);
    const toastJs = new bootstrap.Toast(toastElement);
    toastJs.show();
}