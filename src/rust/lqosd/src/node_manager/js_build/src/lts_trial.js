const sections = [
    "page1", "page2", "alreadyGotIt", "signMeUp"
];

for (let i=1; i<sections.length; i++) {
    $("#"+sections[i]).hide();
}

function hideAll() {
    sections.forEach((s) => { $("#"+s).hide(); });
}

$("#btn1Forward").click(() => {
    hideAll();
    $("#page2").fadeIn();
});

$("#btn2Backward").click(() => {
    hideAll();
    $("#page1").fadeIn();
});

$("#btn2GotIt").click(() => {
    hideAll();
    $("#alreadyGotIt").fadeIn();
});

$("#btn2Forward").click(() => {
    hideAll();
    $("#signMeUp").fadeIn();
});