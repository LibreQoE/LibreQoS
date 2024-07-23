function gather() {
    let details = {
        name: $("#gatherName").val() + ", " + $("#gatherEmail").val(),
        comment: $("#gatherComments").val()
    };
    console.log(details);

    if (details.name === ", " || details.name.indexOf("@")<1) {
        alert("Please enter a name and email address. If you share this, it'll be handy to know who sent it!");
        return;
    }

    $.ajax({
        type: "POST",
        url: "/local-api/gatherSupport",
        data: JSON.stringify(details),
        contentType : 'application/json',
        xhrFields: {
            responseType: 'blob' // to avoid binary data being mangled on charset conversion
        },
        success: function(blob, result, xhr) {
            var filename = "libreqos.support";

            // use HTML5 a[download] attribute to specify filename
            var downloadUrl = URL.createObjectURL(blob);
            var a = document.createElement("a");
            // safari doesn't support this yet
            if (typeof a.download === 'undefined') {
                window.location.href = downloadUrl;
            } else {
                a.href = downloadUrl;
                a.download = filename;
                document.body.appendChild(a);
                a.click();
            }
        }
    })
}

function submit() {
    let details = {
        name: $("#submitName").val() + ", " + $("#submitEmail").val(),
        comment: $("#submitComments").val()
    };
    console.log(details);

    if (details.name === ", " || details.name.indexOf("@")<1) {
        alert("Please enter a name and email address. If you share this, it'll be handy to know who sent it!");
        return;
    }

    $.ajax({
        type: "POST",
        url: "/local-api/submitSupport",
        data: JSON.stringify(details),
        contentType : 'application/json',
        success: function(result) {
            console.log(result);
            alert(result);
        }
    })
}

function sanity() {
    $.get("/local-api/sanity", (data) => {
        console.log(data);
        let html = "<table class='table'><thead><th>Check</th><th>Success?</th><th>Comment</th></thead><tbody>";
        for (let i=0; i<data.results.length; i++) {
            let row = data.results[i];
            html += "<tr>";
            html += "<td>" + row.name + "</td>";
            if (row.success) {
                html += "<td style='color: green'><i class='fa fa-check'></i>";
            } else {
                html += "<td style='color: red'><i class='fa fa-warning'></i>";
            }
            html += "<td>" + row.comments + "</td>";
            html += "</tr>";
        }
        html += "</tbody>";
        $("#configCheck").html(html);

        // Show the modal
        const myModal = new bootstrap.Modal(document.getElementById('sanityModal'), { focus: true });
        myModal.show();
    })
}

// Perform wireups
$("#btnSanity").click(sanity);
$("#btnGather").click(gather);
$("#btnClickSub").click(submit);
