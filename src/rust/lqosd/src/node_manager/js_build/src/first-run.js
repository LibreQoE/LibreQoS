$("#btnCreateUser").on('click', () => {
    let username = ($("#username").val() || "").trim();
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
        beforeSend: () => {
            $("#firstRunError").removeClass("show").addClass("d-none");
        },
        success: () => {
            window.location.href = "/index.html";
        },
        error: (xhr) => {
            const response = xhr && xhr.responseJSON ? xhr.responseJSON : {};
            const reason = response.reason || "";
            if (reason === "already_configured") {
                window.location.href = "/login.html";
                return;
            }

            $("#firstRunErrorText").text(response.message || "Something went wrong");
            $("#firstRunError").removeClass("d-none").addClass("show");
        }
    })
});
