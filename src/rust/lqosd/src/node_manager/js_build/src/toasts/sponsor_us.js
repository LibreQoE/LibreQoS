export function sponsorTag(parentId) {
    $.get("/local-api/ltsCheck", (data) => {
        //console.log(data);
        if (data.action !== "GoodToGo") {
            let div = document.createElement("div");
            let html = "We love working on LibreQoS to make the Internet better. If you love it too, please ";
            html += "<a href=\"https://github.com/sponsors/LibreQoE/\" target='_blank' class='btn btn-sm btn-outline-info'>";
            html += "<i class=\"fa fa-heart\"></i> Sponsor Us on GitHub</a>";
            html += ". By the way, we'll stop asking if you sign up for LTS (Long-Term Stats).";
            div.innerHTML = html;
            div.classList.add("alert", "alert-warning");
            let parent = document.getElementById(parentId);
            parent.appendChild(div);
        }
    });
}