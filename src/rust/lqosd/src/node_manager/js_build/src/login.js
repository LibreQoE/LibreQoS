$("#btnLogin").on('click', () => {
    let username = $("#username").val();
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
            $("#loginError").removeClass("show").addClass("d-none");
        },
        success: () => {
            window.location.href = "/index.html";
        },
        error: () => {
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
