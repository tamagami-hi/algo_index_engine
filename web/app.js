// Everything shown here is derived from the visitor's own clock. Nothing is
// fetched, and no engine, account or position data reaches this page.
(function () {
  "use strict";

  var IST_OFFSET_MINUTES = 330;
  var OPEN_MINUTE = 9 * 60 + 15;
  var CLOSE_MINUTE = 15 * 60 + 30;
  var PRE_OPEN_MINUTE = 9 * 60;

  var DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  var MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
                "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

  function istNow() {
    var now = new Date();
    return new Date(now.getTime() + (IST_OFFSET_MINUTES + now.getTimezoneOffset()) * 60000);
  }

  function pad(value) {
    return value < 10 ? "0" + value : String(value);
  }

  function session(ist) {
    var day = ist.getDay();
    var minute = ist.getHours() * 60 + ist.getMinutes();

    if (day === 0 || day === 6) {
      return { state: "closed", label: "closed \u00b7 weekend", next: nextOpen(ist) };
    }
    if (minute >= OPEN_MINUTE && minute < CLOSE_MINUTE) {
      return { state: "live", label: "open", next: "closes 15:30 IST" };
    }
    if (minute >= PRE_OPEN_MINUTE && minute < OPEN_MINUTE) {
      return { state: "pre", label: "pre-open", next: "opens 09:15 IST" };
    }
    return { state: "closed", label: "closed", next: nextOpen(ist) };
  }

  function nextOpen(ist) {
    var probe = new Date(ist.getTime());
    var minute = ist.getHours() * 60 + ist.getMinutes();

    if (minute >= CLOSE_MINUTE) {
      probe.setDate(probe.getDate() + 1);
    }
    while (probe.getDay() === 0 || probe.getDay() === 6) {
      probe.setDate(probe.getDate() + 1);
    }
    var sameDay = probe.toDateString() === ist.toDateString();
    return (sameDay ? "opens" : "opens " + DAYS[probe.getDay()]) + " 09:15 IST";
  }

  function render() {
    var ist = istNow();

    var time = document.getElementById("ist-time");
    if (time) {
      time.textContent = pad(ist.getHours()) + ":" + pad(ist.getMinutes()) + ":" + pad(ist.getSeconds());
    }

    var date = document.getElementById("ist-date");
    if (date) {
      date.textContent = DAYS[ist.getDay()] + " " + pad(ist.getDate()) + " " +
        MONTHS[ist.getMonth()] + " " + ist.getFullYear();
    }

    var current = session(ist);

    var dot = document.getElementById("session-dot");
    if (dot) {
      dot.className = "dot" + (current.state === "live" ? " live" :
                               current.state === "pre" ? " pre" : "");
    }

    var text = document.getElementById("session-text");
    if (text) { text.textContent = current.label; }

    var next = document.getElementById("session-next");
    if (next) { next.textContent = current.next; }
  }

  render();
  setInterval(render, 1000);
})();
