$("#btnLogin").on('click', () => {
    let username = ($("#username").val() || "").trim();
    let password = $("#password").val();
    if (username === "") {
        alert("You must enter a username");
        return;
    }
    if (password === "") {
        alert("You must enter a password");
        return;
    }

    let login = {
        username: username,
        password: password
    }

    $.ajax({
        type: "POST",
        url: "/doLogin",
        data: JSON.stringify(login),
        contentType: 'application/json',
        beforeSend: () => {
            $("#loginErrorText").html("Login failed. You can manage users via the <code>lqusers</code> CLI tool on the LibreQoS server.");
            $("#loginError").removeClass("show").addClass("d-none");
        },
        success: () => {
            window.location.href = "/index.html";
        },
        error: (xhr) => {
            const response = xhr && xhr.responseJSON ? xhr.responseJSON : {};
            const reason = response.reason || "";
            if (reason === "first_run_required") {
                window.location.href = "/first-run.html";
                return;
            }

            if (reason === "auth_corrupt") {
                $("#loginErrorText").text(response.message || "The auth file is corrupt and must be repaired before anyone can log in.");
            } else if (reason === "invalid_credentials") {
                $("#loginErrorText").text(response.message || "Invalid username or password.");
            } else {
                $("#loginErrorText").html("Login failed. You can manage users via the <code>lqusers</code> CLI tool on the LibreQoS server.");
            }
            $("#loginError").removeClass("d-none").addClass("show");
        }
    })
});

// Add keypress handler for Enter key
$('#username, #password').on('keypress', function(e) {
    if (e.which === 13) {
        e.preventDefault();
        $('#btnLogin').click();
    }
});

// Hide error when typing
$('#username, #password').on('input', function() {
    $("#loginError").fadeOut();
});
