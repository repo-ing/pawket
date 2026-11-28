const form = document.querySelector("#save");
const field = document.querySelector("#link");
const error = document.querySelector("#error");
const setup = document.querySelector("#setup");
const ready = document.querySelector("#ready");
const preference = "pawket-sw-dlt-ready";
let configured = false;
let pending;

try { configured = localStorage.getItem(preference) === "yes"; } catch {}
if (navigator.standalone || window.matchMedia("(display-mode: standalone)").matches) {
  document.querySelector("#install").hidden = true;
}

function openShortcut(url) {
  const shortcut = new URL("shortcuts://run-shortcut");
  shortcut.searchParams.set("name", "SW-DLT");
  shortcut.searchParams.set("input", "text");
  shortcut.searchParams.set("text", url.href);
  window.location.assign(shortcut.href);
}

field.addEventListener("input", () => {
  pending = undefined;
  ready.textContent = "I’ve added the shortcut";
  error.textContent = "";
  field.removeAttribute("aria-invalid");
});

ready.addEventListener("click", () => {
  configured = true;
  try { localStorage.setItem(preference, "yes"); } catch {}
  setup.open = false;
  if (pending) {
    const url = pending;
    pending = undefined;
    openShortcut(url);
  } else {
    field.focus();
  }
});

form.addEventListener("submit", event => {
  event.preventDefault();
  let url;
  try { url = new URL(field.value.trim()); } catch {}
  if (!url || url.protocol !== "https:" || url.username || url.password) {
    error.textContent = "Paste a link that starts with https:// and contains no sign-in details.";
    field.setAttribute("aria-invalid", "true");
    field.focus();
    return;
  }
  if (/\.(mp3|mp4)$/i.test(url.pathname)) {
    window.location.assign(url.href);
  } else if (configured) {
    openShortcut(url);
  } else {
    pending = url;
    setup.open = true;
    ready.textContent = "I’ve added it · Continue";
    setup.querySelector("summary").focus();
  }
});
