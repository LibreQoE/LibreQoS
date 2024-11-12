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
        success: () => {
            window.location.href = "/index.html";
        },
        error: () => {
            alert("Login Incorrect");
        }
    })
});