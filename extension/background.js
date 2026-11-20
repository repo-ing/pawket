// Chrome API bridge only: adds the right-click menu and opens the Pawket
// popup with the chosen link. Inspecting and saving happen in Rust.
const MENU_ID = "pawket-save";
const POPUP_SIZE = { width: 396, height: 640 };

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: MENU_ID,
    title: "Save with Pawket",
    contexts: ["page", "link", "video", "audio", "image"],
  });
});

const isHttps = (value) => typeof value === "string" && value.startsWith("https://");

chrome.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId !== MENU_ID) return;
  // Prefer what was clicked; fall back to the page (players often use blob: sources).
  const target = [info.linkUrl, info.srcUrl, info.pageUrl].find(isHttps);
  if (!target) return;
  const page = chrome.runtime.getURL(`popup.html?url=${encodeURIComponent(target)}`);
  chrome.windows.create({ url: page, type: "popup", ...POPUP_SIZE });
});
