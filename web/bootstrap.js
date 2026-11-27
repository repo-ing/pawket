import init, { start } from "./pkg/pawket_browser.js";

await init();
start("web", "", false);

const folderName = document.querySelector("#folder-name");
const folderStatus = document.querySelector("#folder-status");
const folderButtons = document.querySelectorAll(".save-folder button");
async function folderAction(action) {
  folderButtons.forEach(button => button.disabled = true);
  try {
    const response = await fetch("/v1/folder/" + action, {method:"POST",
      headers:{"X-Pawket-Token":document.querySelector('meta[name="pawket-token"]').content}});
    const data = await response.json();
    if (!response.ok) throw Error(data.error?.message || "Could not access the folder.");
    folderName.textContent = data.path.split(/[/\\]/).filter(Boolean).slice(-2).join("/");
    folderName.title = data.path;
    folderStatus.textContent = "";
  } catch (error) { folderStatus.textContent = error.message; }
  finally { folderButtons.forEach(button => button.disabled = false); }
}
document.querySelector("#choose-folder").onclick = () => folderAction("choose");
document.querySelector("#open-folder").onclick = () => folderAction("open");
folderAction("status");
