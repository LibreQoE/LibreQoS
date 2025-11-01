use std::borrow::Cow;
use std::ffi::{CStr, CString};

use cursive::view::{Nameable, Resizable};
use cursive::views::{Checkbox, Dialog, LinearLayout, Menubar, SelectView, TextView};
use cursive::Cursive;
use nix::libc::if_nametoindex;
mod system_info;

fn bridge_menu(s: &mut Cursive) {
	let mut select_internet = SelectView::new();
	let mut select_lan = SelectView::new();

	system_info::get_nic_list().into_iter().for_each(|iface| {
		let name = iface.interface_name.clone();
		let name_c_str = CString::new(iface.interface_name).unwrap();
		let idx = unsafe { if_nametoindex(name_c_str.as_ptr()) };
		
		select_internet.add_item(name.clone(), idx);
		select_lan.add_item(name, idx);
	});

	s.pop_layer();
	s.add_layer(
		Dialog::new()
		.title("Bridge Settings")
		.content(
			LinearLayout::horizontal()
			.child(
				LinearLayout::vertical()
				.child(LinearLayout::horizontal()
					.child(Checkbox::new())
					.child(TextView::new("Use XDP Bridge"))
				)
				.child(TextView::new("\nThe XDP bridge is generally faster\non non-nVidia devices"))
			)
			.child(TextView::new(" "))
			.child(
				LinearLayout::vertical()
				.child(TextView::new("Internet Facing"))
				.child(select_internet)
			)
			.child(TextView::new(" "))
			.child(
				LinearLayout::vertical()
				.child(TextView::new("Core Facing"))
				.child(select_lan)
			)
		).full_screen()
	);
}

fn main_menu(s: &mut Cursive) {
	s.pop_layer();
	// s.add_layer(
	// 	Dialog::text(GREETINGS)
    //     .title("LibreQoS Setup System 2.0")
	// 	.button("Bridge Configuration", bridge_menu)
	// );
	s.add_layer(Menubar::new()
	.add_leaf("File", bridge_menu));
}

const GREETINGS: &str = r#"Press Q to quit."#;

fn main() {
	let mut siv = cursive::default();
	siv.add_global_callback('q', |s| s.quit());

	main_menu(&mut siv);

	Menubar::new()
	.add_leaf("Bridge", cb)

	siv.run();
}