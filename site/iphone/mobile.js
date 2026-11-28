const form = document.querySelector("#save");
const field = document.querySelector("#link");
const error = document.querySelector("#error");
if (navigator.standalone || window.matchMedia("(display-mode: standalone)").matches) {
  document.querySelector("#install").hidden = true;
}
form.addEventListener("submit", event => {
  event.preventDefault();
  let url;
  try {
    url = new URL(field.value.trim());
  } catch {
    error.textContent = "Paste a full HTTPS link.";
    return;
  }
  if (url.protocol !== "https:" || url.username || url.password) {
    error.textContent = "Use a public HTTPS link without embedded credentials.";
    return;
  }
  error.textContent = "";
  if (/\.(mp3|mp4)$/i.test(url.pathname)) {
    window.location.assign(url.href);
    return;
  }
  const shortcut = new URL("shortcuts://run-shortcut");
  shortcut.searchParams.set("name", "SW-DLT");
  shortcut.searchParams.set("input", "text");
  shortcut.searchParams.set("text", url.href);
  window.location.assign(shortcut.href);
});
