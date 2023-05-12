class Wizard {
    #currentTab = 0;
    constructor() {

    }
    showTab(n) {
        var x = document.getElementsByClassName("tab");
        x[n].style.display = "block";
        if (n == 0)
            document.getElementById("prevBtn").style.display = "none";
        else
            document.getElementById("prevBtn").style.display = "inline";
        if (n == (x.length - 1))
            document.getElementById("nextBtn").innerHTML = "Submit";
        else
            document.getElementById("nextBtn").innerHTML = "Next";
        this.fixStepIndicator(n)
    }
    nextPrev(n) {
        var x = document.getElementsByClassName("tab");
        if (n == 1 && !validateForm()) return false;
        x[this.#currentTab].style.display = "none";
        this.#currentTab = this.#currentTab + n;
        if (this.#currentTab >= x.length) {
            document.getElementById("regForm").submit();
            return false;
        }
        this.showTab(this.#currentTab);
    }
    validateForm() {
        var x, y, i, valid = true;
        x = document.getElementsByClassName("tab");
        y = x[this.#currentTab].getElementsByTagName("input");
        for (i = 0; i < y.length; i++) {
            if (y[i].value == "") {
                y[i].className += " invalid";
                valid = false;
            }
        }
        if (valid)
            document.getElementsByClassName("step")[this.#currentTab].className += " finish";
        return valid;
    }
    fixStepIndicator(n) {
        var i, x = document.getElementsByClassName("step");
        for (i = 0; i < x.length; i++)
            x[i].className = x[i].className.replace(" active", "");
        x[n].className += " active";
    }
}