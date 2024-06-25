// Creates a dark background that covers the whole window
export function darkBackground(id) {
    let darken = document.createElement("div");
    darken.id = id;
    darken.style.zIndex = 200;
    darken.style.position = "absolute";
    darken.style.top = "0px";
    darken.style.bottom = "0px";
    darken.style.left = "0px";
    darken.style.right = "0px";
    darken.style.background = "rgba(1, 1, 1, 0.75)";
    return darken;
}

// Creates a div that sits happily atop the window
export function modalContent(closeTargetId) {
    let content = document.createElement("div");
    content.style.zIndex = 210;
    content.style.position = "absolute";
    content.style.top = "10%";
    content.style.bottom = "10%";
    content.style.left = "10%";
    content.style.right = "10%";
    content.style.maxWidth = "500px";
    content.style.maxHeight = "500px";
    content.style.background = "#eee";
    content.style.padding = "10px";
    content.appendChild(closeButton(closeTargetId));
    return content;
}

function closeButton(closeTargetId) {
    let closeDiv = document.createElement("div");
    closeDiv.style.position = "absolute";
    closeDiv.style.right = "0";
    closeDiv.style.top = "0";
    closeDiv.style.width = "25px";
    closeDiv.style.height = "25px";
    let close = document.createElement("button");
    close.classList.add("btn", "btn-sm", "btn-danger");
    close.innerText = "X";
    close.type = "button";
    close.onclick = () => { $("#" + closeTargetId).remove() };
    closeDiv.appendChild(close);
    return closeDiv;
}