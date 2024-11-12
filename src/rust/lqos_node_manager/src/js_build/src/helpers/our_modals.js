// Creates a dark background that covers the whole window
export function darkBackground(id) {
    let darken = document.createElement("div");
    darken.id = id;
    darken.classList.add("darkenBackground")
    return darken;
}

// Creates a div that sits happily atop the window
export function modalContent(closeTargetId) {
    let content = document.createElement("div");
    content.classList.add("myModal");
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