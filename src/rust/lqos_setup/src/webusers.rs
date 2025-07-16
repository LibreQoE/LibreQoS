//! Web Users editor for LibreQoS setup console
use cursive::{
    view::{Nameable, Resizable},
    views::{Button, Dialog, EditView, LinearLayout, SelectView, TextView},
    Cursive,
};
use lqos_config::{WebUsers, UserRole};

/// Shows and manages the list of web users.
pub fn webusers_menu(s: &mut Cursive) {
    // Load or create the web users config
    let webusers = match WebUsers::load_or_create() {
        Ok(wu) => wu,
        Err(e) => {
            s.add_layer(Dialog::info(format!("Failed to load web users: {e}")));
            return;
        }
    };

    // Prepare the SelectView with usernames
    let select_view = SelectView::<String>::new()
        .with_all(
            webusers
                .get_users()
                .iter()
                .map(|user| (user.username.clone(), user.username.clone())),
        )
        .with_name("web_users")
        .fixed_width(30);

    // Layout for user management
    let layout = LinearLayout::horizontal()
        .child(
            LinearLayout::vertical()
                .child(TextView::new("Web Users:"))
                .child(select_view)
                .child(Button::new("Remove Selected", move |s| {
                    let mut webusers = match WebUsers::load_or_create() {
                        Ok(wu) => wu,
                        Err(e) => {
                            s.add_layer(Dialog::info(format!("Failed to load web users: {e}")));
                            return;
                        }
                    };
                    let (selected, username): (usize, String) = s.call_on_name("web_users", |view: &mut SelectView<String>| {
                        view.selected_id()
                            .and_then(|selected| {
                                view.get_item(selected)
                                    .map(|(name, _)| (selected, name.to_string()))
                            })
                    }).unwrap_or(None).unwrap_or((0, String::new()));

                    if !username.is_empty() {
                        match webusers.remove_user(&username) {
                            Ok(_) => {
                                s.call_on_name("web_users", |view: &mut SelectView<String>| {
                                    view.remove_item(selected);
                                });
                            }
                            Err(e) => {
                                s.add_layer(Dialog::info(format!("Failed to remove user: {e}")));
                            }
                        }
                    }
                })),
        )
        .child(
            LinearLayout::vertical()
                .child(TextView::new("Add New User:"))
                .child(
                    LinearLayout::horizontal()
                        .child(TextView::new("Username: ").fixed_width(12))
                        .child(
                            EditView::new()
                                .with_name("new_username")
                                .fixed_width(20),
                        ),
                )
                .child(
                    LinearLayout::horizontal()
                        .child(TextView::new("Password: ").fixed_width(12))
                        .child(
                            EditView::new()
                                .secret()
                                .with_name("new_password")
                                .fixed_width(20),
                        ),
                )
                .child(
                    LinearLayout::horizontal()
                        .child(TextView::new("Role:     ").fixed_width(12))
                        .child({
                            let mut role_edit = EditView::new();
                            role_edit.set_content("Admin");
                            role_edit.with_name("new_role").fixed_width(20)
                        }),
                )
                .child(Button::new("Add User", |s| {
                    let username = s
                        .call_on_name("new_username", |view: &mut EditView| view.get_content())
                        .unwrap()
                        .to_string();
                    let password = s
                        .call_on_name("new_password", |view: &mut EditView| view.get_content())
                        .unwrap()
                        .to_string();
                    let role_str = s
                        .call_on_name("new_role", |view: &mut EditView| view.get_content())
                        .unwrap()
                        .to_string();
                    let role = UserRole::from(role_str.as_str());
                    let mut webusers = match WebUsers::load_or_create() {
                        Ok(wu) => wu,
                        Err(e) => {
                            s.add_layer(Dialog::info(format!("Failed to load web users: {e}")));
                            return;
                        }
                    };
                    match webusers.add_or_update_user(&username, &password, role) {
                        Ok(_) => {
                            s.call_on_name("web_users", |view: &mut SelectView<String>| {
                                view.add_item(username.clone(), username.clone());
                            });
                        }
                        Err(e) => {
                            s.add_layer(Dialog::info(format!("Failed to add user: {e}")));
                        }
                    }
                }))
                .child(TextView::new("Fill all fields and press Enter or Add User")),
        );

    s.add_layer(
        Dialog::around(layout)
            .title("Web Users")
            .button("Change Password", |s| {
                // Show dialog to change password for selected user
                let selected = s
                    .call_on_name("web_users", |view: &mut SelectView<String>| {
                        view.selection().map(|arc| (*arc).clone())
                    })
                    .flatten();
                if let Some(username) = selected {
                    s.add_layer(
                        Dialog::new()
                            .title(format!("Change Password for {}", username))
                            .content(
                                EditView::new()
                                    .secret()
                                    .with_name("change_password")
                                    .fixed_width(20),
                            )
                            .button("OK", move |s| {
                                let password = s
                                    .call_on_name("change_password", |view: &mut EditView| {
                                        view.get_content()
                                    })
                                    .unwrap()
                                    .to_string();
                                let mut webusers = match WebUsers::load_or_create() {
                                    Ok(wu) => wu,
                                    Err(e) => {
                                        s.add_layer(Dialog::info(format!(
                                            "Failed to load web users: {e}"
                                        )));
                                        return;
                                    }
                                };
                                // Default to Admin role for password change (could be improved)
                                let user = webusers
                                    .get_users()
                                    .into_iter()
                                    .find(|u| u.username == username)
                                    .unwrap();
                                let role = user.role;
                                match webusers.add_or_update_user(&username, &password, role) {
                                    Ok(_) => {
                                        s.pop_layer();
                                    }
                                    Err(e) => {
                                        s.add_layer(Dialog::info(format!(
                                            "Failed to change password: {e}"
                                        )));
                                    }
                                }
                            })
                            .button("Cancel", |s| {
                                s.pop_layer();
                            }),
                    );
                } else {
                    s.add_layer(Dialog::info("No user selected."));
                }
            })
            .button("OK", |s| {
                s.pop_layer();
            })
            .full_screen(),
    );
}