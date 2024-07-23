const sponsorBtn = "<a href=\"https://github.com/sponsors/LibreQoE/\" target='_blank' class='text-primary-emphasis'><i class=\"fa fa-heart\"></i> Sponsor Us on GitHub</a>";

export function sponsorTag(parentId) {
    $.get("/local-api/ltsCheck", (data) => {
        $.get("/local-api/deviceCount", (counts) => {
            //console.log(data);
            if (data.action !== "GoodToGo") {
                let div = document.createElement("div");
                let random = Math.floor(Math.random() * 5) + 1;
                if (random === 1) {
                    let html = "We love working on LibreQoS to make the Internet better. If you love it too, please ";
                    html += sponsorBtn;
                    html += ". By the way, we'll stop asking if you sign up for LTS (Long-Term Stats).";
                    div.innerHTML = html;
                } else if (random === 2) {
                    let cost = Math.max(100, counts.shaped_devices) * 0.6;
                    let html = "Other QoS providers might be charging you as much as $" + cost.toFixed(2) + " per month. We like eating, too! Why not ";
                    html += sponsorBtn;
                    html += ". By the way, we'll stop asking if you sign up for LTS (Long-Term Stats).";
                    div.innerHTML = html;
                } else if (random === 3) {
                    let html = "Open Source is a labor of love. If you love LibreQoS, please ";
                    html += sponsorBtn;
                    html += ". By the way, we'll stop asking if you sign up for LTS (Long-Term Stats).";
                    div.innerHTML = html;
                } else if (random === 4) {
                    let html = counts.shaped_devices + " devices on your network are using LibreQoS. If we're helping, please ";
                    html += sponsorBtn;
                    html += ". By the way, we'll stop asking if you sign up for LTS (Long-Term Stats).";
                    div.innerHTML = html;
                } else if (random === 5) {
                    let html = "$150 will keep a developer in Ramen for a month! ";
                    html += sponsorBtn;
                    html += ". By the way, we'll stop asking if you sign up for LTS (Long-Term Stats).";
                    div.innerHTML = html;
                }
                div.classList.add("alert", "alert-warning");
                let parent = document.getElementById(parentId);
                parent.appendChild(div);
            }
        });
    });
}