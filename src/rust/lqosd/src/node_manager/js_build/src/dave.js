export function inMemoriam() {
    if (!localStorage.getItem('displayedDaveMemorial')) {
        displayDaveMemorial();
    }
}

export function displayDaveMemorial() {
    // Create modal elements using Bootstrap classes
    const modalHTML = `
            <div class="modal fade" id="daveModal" tabindex="-1" aria-labelledby="daveModalLabel" aria-hidden="true">
                <div class="modal-dialog modal-fullscreen">
                    <div class="modal-content bg-dark text-light">
                        <div class="modal-header border-secondary">
                            <h2 class="modal-title fs-1" id="daveModalLabel">In Loving Memory of Dave Täht</h2>
                            <button type="button" class="btn-close btn-close-white" data-bs-dismiss="modal" aria-label="Close"></button>
                        </div>
                        <div class="modal-body text-center">
                            <div class="row mb-4">
                                <div class="col-12">
                                    <p class="lead">We're devastated to announce that Dave Täht has passed away.</p>
                                    <p>Dave was an amazing man, helping the world with FQ-CoDel and CAKE, fighting bufferbloat and trying to make the world a better place. Always willing to help, and without him - LibreQoS (and the other QoE solutions out there) wouldn't exist.</p>
                                    <p>Dave was an inspiration, and we all miss him. We're reaching out to family and close friends to see if there's anything we can do to help.</p>
                                </div>
                            </div>
                            <div class="row row-cols-1 row-cols-md-3 g-4">
                                <div class="col">
                                    <img src="dave/dave1.jpg" 
                                         class="img-fluid rounded" 
                                         style="object-fit: cover; width: 75%; height: 75%"
                                         alt="Dave Täht smiling">
                                </div>
                                <div class="col">
                                    <img src="dave/dave2.jpg" 
                                         class="img-fluid rounded" 
                                         style="object-fit: cover; width: 75%; height: 75%"
                                         alt="Dave Täht with Robert and Frank, in Las Vegas.">
                                </div>
                                <div class="col">
                                    <img src="dave/dave3.jpg" 
                                         class="img-fluid rounded" 
                                         style="object-fit: cover; width: 75%; height: 75%"
                                         alt="Dave Täht in a beret, with a guitar, at Wispapalooza">
                                </div>
                            </div>
                        </div>
                        <div class="modal-footer border-secondary">
                            <button type="button" class="btn btn-outline-light" data-bs-dismiss="modal">Close</button>
                        </div>
                    </div>
                </div>
            </div>
        `;

    // Add modal to DOM
    document.body.insertAdjacentHTML('beforeend', modalHTML);

    // Initialize and show modal
    const daveModal = new bootstrap.Modal(document.getElementById('daveModal'));
    daveModal.show();

    // Set flag when modal closes
    document.getElementById('daveModal').addEventListener('hidden.bs.modal', () => {
        localStorage.setItem('displayedDaveMemorial', 'true');
        document.getElementById('daveModal').remove();
    });
}