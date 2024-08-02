$("#btnCreateUser").on('click', () => {
    let username = $("#username").val();
    let password = $("#password").val();
    let anon = document.getElementById("allowAnonymous").checked;
    if (username === "") {
        alert("You must enter a username");
        return;
    }
    if (password === "") {
        alert("You must enter a password");
        return;
    }

    let login = {
        allow_anonymous: anon,
        username: username,
        password: password
    }
    $.ajax({
        type: "POST",
        url: "/firstLogin",
        data: JSON.stringify(login),
        contentType: 'application/json',
        success: () => {
            window.location.href = "/index.html";
        },
        error: () => {
            alert("Something went wrong");
        }
    })
});