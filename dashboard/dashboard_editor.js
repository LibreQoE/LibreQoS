/**
 * Opens a full screen modal that lets the user edit dashboard items.
 * @param {Array} initialElements - Array of objects representing current dashboard items. Each object should have a "name" and a "size" (1–12).
 * @param {Array} availableElements - Array of objects representing available elements to add (each with a "name" and default "size").
 * @param {Function} callback - A function that will be called with the new array of dashboard items when the user clicks "Done".
 */
export function openDashboardEditor(initialElements, availableElements, callback) {
    // Build modal HTML with a fullscreen modal, a grid area and an available panel.
    var modalHtml = `
  <div class="modal fade" id="dashboardEditorModal" tabindex="-1" aria-labelledby="dashboardEditorModalLabel" aria-hidden="true">
    <div class="modal-dialog modal-fullscreen">
      <div class="modal-content">
        <div class="modal-header">
          <h5 class="modal-title" id="dashboardEditorModalLabel">Dashboard Editor</h5>
          <button type="button" class="btn-close" data-bs-dismiss="modal" aria-label="Close"></button>
        </div>
        <div class="modal-body">
          <div class="container-fluid">
            <div class="row">
              <!-- Main grid area for dashboard items -->
              <div class="col-md-9">
                <div id="dashboardGrid" class="row gy-3">
                  <!-- Dashboard items will be injected here -->
                </div>
              </div>
              <!-- Panel for available elements -->
              <div class="col-md-3 border-start">
                <h6>Available Elements</h6>
                <ul id="availableList" class="list-group">
                  <!-- Available elements will be injected here -->
                </ul>
              </div>
            </div>
          </div>
        </div>
        <div class="modal-footer">
          <button id="dashboardDone" type="button" class="btn btn-primary">Done</button>
          <button type="button" class="btn btn-secondary" data-bs-dismiss="modal">Cancel</button>
        </div>
      </div>
    </div>
  </div>
  `;

    // Append the modal HTML to the body.
    $('body').append(modalHtml);

    // Render the current dashboard items in the grid.
    function renderDashboard() {
        var $grid = $('#dashboardGrid');
        $grid.empty();
        initialElements.forEach(function(item, index) {
            var itemHtml = `
      <div class="dashboard-item col-${item.size}" data-index="${index}" data-size="${item.size}" data-name="${item.name}">
        <div class="card">
          <div class="card-body p-2">
            <div class="d-flex justify-content-between align-items-center">
              <span>${item.name}</span>
              <div>
                <button class="btn btn-sm btn-outline-secondary decrease-width">-</button>
                <button class="btn btn-sm btn-outline-secondary increase-width">+</button>
                <button class="btn btn-sm btn-outline-danger delete-item">x</button>
              </div>
            </div>
          </div>
        </div>
      </div>
      `;
            $grid.append(itemHtml);
        });
    }

    // Render available elements in the side panel with accordion
    function renderAvailable() {
        var $available = $('#availableList');
        $available.empty();

        // Group elements by category
        const categories = availableElements.reduce((acc, item) => {
            const category = item.category || 'Uncategorized';
            if (!acc[category]) acc[category] = [];
            acc[category].push(item);
            return acc;
        }, {});

        // Build accordion HTML with instructions
        const accordionHTML = `
            <div class="alert alert-info d-flex align-items-center gap-2 mb-3" role="alert">
                <i class="bi bi-info-circle"></i>
                Drag new widgets to the dashboard
            </div>
            <div class="accordion" id="availableAccordion">
                ${Object.entries(categories).map(([category, items], index) => `
                    <div class="accordion-item">
                        <h2 class="accordion-header">
                            <button class="accordion-button ${index === 0 ? '' : 'collapsed'}" 
                                type="button" 
                                data-bs-toggle="collapse" 
                                data-bs-target="#cat-${index}" 
                                aria-expanded="${index === 0 ? 'true' : 'false'}" 
                                aria-controls="cat-${index}">
                                ${category}
                            </button>
                        </h2>
                        <div id="cat-${index}" 
                            class="accordion-collapse collapse ${index === 0 ? 'show' : ''}" 
                            data-bs-parent="#availableAccordion">
                            <div class="accordion-body p-0">
                                <ul class="list-group">
                                    ${items.map(item => `
                                        <li class="list-group-item available-item" 
                                            data-name="${item.name}" 
                                            data-size="${item.size}">
                                            ${item.name}
                                        </li>
                                    `).join('')}
                                </ul>
                            </div>
                        </div>
                    </div>
                `).join('')}
            </div>
        `;
        $available.html(accordionHTML);
    }

    renderDashboard();
    renderAvailable();

    // Initialize SortableJS for the dashboard grid.
    // This allows rearranging within the grid and also accepting items from the available list.
    var dashboardSortable = new Sortable(document.getElementById('dashboardGrid'), {
        animation: 150,
        group: {
            name: 'shared',
            pull: true,
            put: true
        },
        onAdd: function (evt) {
            // When an item is dropped into the grid (from the available list),
            // replace it with a fully rendered dashboard item.
            var $item = $(evt.item);
            var name = $item.data('name');
            var size = $item.data('size');
            $item.replaceWith(`
        <div class="dashboard-item col-${size}" data-size="${size}" data-name="${name}">
          <div class="card">
            <div class="card-body p-2">
              <div class="d-flex justify-content-between align-items-center">
                <span>${name}</span>
                <div>
                  <button class="btn btn-sm btn-outline-secondary decrease-width">-</button>
                  <button class="btn btn-sm btn-outline-secondary increase-width">+</button>
                  <button class="btn btn-sm btn-outline-danger delete-item">x</button>
                </div>
              </div>
            </div>
          </div>
        </div>
      `);
        }
    });

    // Initialize SortableJS for all available elements lists in the accordion
    document.querySelectorAll('.accordion-body .list-group').forEach(list => {
        new Sortable(list, {
            animation: 150,
            group: {
                name: 'shared',
                pull: 'clone',
                put: false
            },
            sort: false
        });
    });

    // Handle delete action for dashboard items.
    $('#dashboardGrid').on('click', '.delete-item', function() {
        $(this).closest('.dashboard-item').remove();
    });

    // Allow increasing the width of an item (up to 12).
    $('#dashboardGrid').on('click', '.increase-width', function() {
        var $item = $(this).closest('.dashboard-item');
        var size = parseInt($item.data('size'), 10);
        if (size < 12) {
            size++;
            $item.data('size', size);
            // Remove previous col-* class and add the new one.
            $item.removeClass(function(index, className) {
                return (className.match(/(^|\s)col-\S+/g) || []).join(' ');
            }).addClass('col-' + size);
        }
    });

    // Allow decreasing the width of an item (minimum 1).
    $('#dashboardGrid').on('click', '.decrease-width', function() {
        var $item = $(this).closest('.dashboard-item');
        var size = parseInt($item.data('size'), 10);
        if (size > 1) {
            size--;
            $item.data('size', size);
            $item.removeClass(function(index, className) {
                return (className.match(/(^|\s)col-\S+/g) || []).join(' ');
            }).addClass('col-' + size);
        }
    });

    // When the "Done" button is clicked, collect the new layout and call the callback.
    $('#dashboardDone').on('click', function() {
        var newElements = [];
        $('#dashboardGrid .dashboard-item').each(function() {
            var $el = $(this);
            newElements.push({
                name: $el.data('name'),
                size: parseInt($el.data('size'), 10)
            });
        });
        // Hide and remove the modal.
        $('#dashboardEditorModal').modal('hide').remove();
        // Pass the new array back to the caller.
        callback(newElements);
    });

    // Show the modal using Bootstrap's modal API.
    var modalEl = document.getElementById('dashboardEditorModal');
    var modal = new bootstrap.Modal(modalEl, {
        backdrop: 'static',
        keyboard: false
    });
    modal.show();

    // Clean up when the modal is hidden.
    $('#dashboardEditorModal').on('hidden.bs.modal', function() {
        $(this).remove();
    });
}
