$(document).ready(() => {
    loadUsers();
    
    // Handle add user form submission
    $('#add-user-form').on('submit', function(e) {
        e.preventDefault();
        const username = $('#username').val();
        const password = $('#password').val();
        const role = $('#role').val();
        
        $.ajax({
            type: "POST",
            url: "/local-api/addUser",
            data: JSON.stringify({ 
                username: username,
                password: password,
                role: role 
            }),
            contentType: 'application/json',
            success: () => {
                $('#username').val('');
                $('#password').val('');
                loadUsers();
            },
            error: () => {
                alert('Failed to add user');
            }
        });
    });

    // Handle edit user form submission
    $('#save-user-changes').on('click', function() {
        const username = $('#edit-username').val();
        const password = $('#edit-password').val();
        const role = $('#edit-role').val();
        
        $.ajax({
            type: "POST",
            url: "/local-api/updateUser",
            data: JSON.stringify({ 
                username: username,
                password: password,
                role: role 
            }),
            contentType: 'application/json',
            success: () => {
                $('#editUserModal').modal('hide');
                loadUsers();
            },
            error: () => {
                alert('Failed to update user');
            }
        });
    });
});

function loadUsers() {
    $.get('/local-api/getUsers', (users) => {
        const userList = $('#users-list');
        userList.empty();
        
        if (users.length === 0) {
            userList.html('<div class="alert alert-info">No users found</div>');
            return;
        }

        const table = $('<table class="table table-striped">')
            .append('<thead><tr><th>Username</th><th>Role</th><th>Actions</th></tr></thead>');
        const tbody = $('<tbody>');
        
        users.forEach(user => {
            const row = $('<tr>')
                .append(`<td>${user.username}</td>`)
                .append(`<td>${user.role}</td>`)
                .append(`<td>
                    <button class="btn btn-sm btn-primary edit-user" data-username="${user.username}">
                        <i class="fa fa-edit"></i> Edit
                    </button>
                    <button class="btn btn-sm btn-danger delete-user" data-username="${user.username}">
                        <i class="fa fa-trash"></i> Delete
                    </button>
                </td>`);
            
            tbody.append(row);
        });

        table.append(tbody);
        userList.append(table);

        // Attach edit handlers
        $('.edit-user').on('click', function() {
            const username = $(this).data('username');
            const user = users.find(u => u.username === username);
            $('#edit-username').val(user.username);
            $('#edit-role').val(user.role);
            $('#editUserModal').modal('show');
        });

        // Attach delete handlers
        $('.delete-user').on('click', function() {
            if (confirm('Are you sure you want to delete this user?')) {
                const username = $(this).data('username');
                $.ajax({
                    type: "POST",
                    url: "/local-api/deleteUser",
                    data: JSON.stringify({ username: username }),
                    contentType: 'application/json',
                    success: () => {
                        loadUsers();
                    },
                    error: () => {
                        alert('Failed to delete user');
                    }
                });
            }
        });
    }).fail(() => {
        $('#users-list').html('<div class="alert alert-danger">Failed to load users</div>');
    });
}
