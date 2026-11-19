import {
    addUser,
    deleteUser,
    getUsers,
    renderConfigMenu,
    updateUser,
} from "./config/config_helper";
import {t} from "./i18n/index.js";

// The role column shows a label for the role the server stores; an
// unrecognised role keeps the token it came with.
function roleLabel(role) {
    // translatable: false — "Admin" is the role token the server stores and this branch compares, never rendered
    if (role === "Admin") return t("config.users.role.admin");
    if (role === "ReadOnly") return t("config.users.role.read_only");
    return role;
}

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
            alert(t("config.users.validation.username_required"));
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
                    alert(msg && msg.message ? msg.message : t("config.users.error.add"));
                }
            },
            (e) => {
                alert(t("config.users.error.add"));
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
                    alert(msg && msg.message ? msg.message : t("config.users.error.update"));
                }
            },
            () => {
                alert(t("config.users.error.update"));
            },
        );
    });
});

function loadUsers() {
    getUsers((users) => {
        const userList = $('#users-list');
        userList.empty();
        
        if (users.length === 0) {
            userList.html(`<div class="alert alert-info">${t("config.users.empty")}</div>`);
            return;
        }

        const tableWrap = $('<div class="table-responsive lqos-table-wrap">');
        const table = $('<table class="lqos-table lqos-table-compact mb-0">')
            .append(`<thead><tr><th>${t("config.users.username")}</th><th>${t("config.users.role")}</th><th>${t("common.table.actions")}</th></tr></thead>`);
        const tbody = $('<tbody>');
        
        users.forEach(user => {
            const actions = $('<td>');
            const editButton = $('<button type="button" class="btn btn-sm btn-primary edit-user">')
                .data('username', user.username)
                .append($('<i class="fa fa-edit">'))
                .append(document.createTextNode(` ${t("config.users.edit.action")}`));
            const deleteButton = $('<button type="button" class="btn btn-sm btn-danger delete-user">')
                .data('username', user.username)
                .append($('<i class="fa fa-trash">'))
                .append(document.createTextNode(` ${t("common.action.delete")}`));

            actions.append(editButton, ' ', deleteButton);

            const row = $('<tr>')
                .append($('<td>').text(user.username))
                .append($('<td>').text(roleLabel(user.role)))
                .append(actions);
            
            tbody.append(row);
        });

        table.append(tbody);
        tableWrap.append(table);
        userList.append(tableWrap);

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
            if (confirm(t("config.users.confirm_delete"))) {
                const username = $(this).data('username');
                deleteUser(
                    { username: username },
                    (msg) => {
                        if (msg && msg.ok) {
                            loadUsers();
                        } else {
                            alert(msg && msg.message ? msg.message : t("config.users.error.delete"));
                        }
                    },
                    (e) => {
                        console.error(e);
                        alert(t("config.users.error.delete"));
                    },
                );
            }
        });
    }, () => {
        $('#users-list').html(`<div class="alert alert-danger">${t("config.users.error.load")}</div>`);
    });
}
