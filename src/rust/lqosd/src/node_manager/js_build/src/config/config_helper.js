export function loadConfig(onComplete) {
    $.get("/local-api/getConfig", (data) => {
        window.config = data;
        onComplete();
    });
}

export function saveConfig(onComplete) {
    $.ajax({
        type: "POST",
        url: "/local-api/updateConfig",
        data: JSON.stringify(window.config),
        contentType: 'application/json',
        success: () => {
            onComplete();
        },
        error: () => {
            alert("That didn't work");
        }
    });
}