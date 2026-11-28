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
  if (url.protocol !== "https:" || url.username || url.password || !/\.(mp3|mp4)$/i.test(url.pathname)) {
    error.textContent = "Use an HTTPS file link ending in .mp3 or .mp4.";
    return;
  }
  error.textContent = "";
  window.location.assign(url.href);
});
