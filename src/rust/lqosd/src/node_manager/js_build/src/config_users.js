import {
    addUser,
    deleteUser,
    getUsers,
    renderConfigMenu,
    updateUser,
} from "./config/config_helper";

$(document).ready(() => {
    // Render the configuration menu
    renderConfigMenu('users');
    
    loadUsers();
    
    // Handle add user form submission
    $('#add-user-form').on('submit', function(e) {
        e.preventDefault();
        const username = $('#add-username').val().trim();
        const password = $('#password').val();
        const role = $('#role').val();
        
        if (!username) {
            alert('Username cannot be empty');
            return;
        }
        
        addUser(
            {
                username: username,
                password: password,
                role: role,
            },
            (msg) => {
                if (msg && msg.ok) {
                    $('#add-username').val('');
                    $('#password').val('');
                    loadUsers();
                } else {
                    alert(msg && msg.message ? msg.message : 'Failed to add user');
                }
            },
            (e) => {
                alert('Failed to add user');
                console.log(e);
            },
        );
    });

    // Handle edit user form submission
    $('#save-user-changes').on('click', function() {
        const username = $('#edit-username').val();
        const password = $('#edit-password').val();
        const role = $('#edit-role').val();

        const payload = {
            username: username,
            role: role
        };

        // Only send password if the field is non-empty; leaving it blank
        // preserves the existing password on the server.
        if (password && password.trim().length > 0) {
            payload.password = password;
        }

        updateUser(
            payload,
            (msg) => {
                if (msg && msg.ok) {
                    $('#edit-password').val('');
                    $('#editUserModal').modal('hide');
                    loadUsers();
                } else {
                    alert(msg && msg.message ? msg.message : 'Failed to update user');
                }
            },
            () => {
                alert('Failed to update user');
            },
        );
    });
});

function loadUsers() {
    getUsers((users) => {
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
            $('#edit-password').val('');
            $('#edit-username').val(user.username);
            $('#edit-role').val(user.role);
            $('#editUserModal').modal('show');
        });

        // Attach delete handlers
        $('.delete-user').on('click', function() {
            if (confirm('Are you sure you want to delete this user?')) {
                const username = $(this).data('username');
                deleteUser(
                    { username: username },
                    (msg) => {
                        if (msg && msg.ok) {
                            loadUsers();
                        } else {
                            alert(msg && msg.message ? msg.message : 'Failed to delete user');
                        }
                    },
                    (e) => {
                        console.error(e);
                        alert('Failed to delete user');
                    },
                );
            }
        });
    }, () => {
        $('#users-list').html('<div class="alert alert-danger">Failed to load users</div>');
    });
}
