// Stand-in for the Nuxt entry chunk: requests one more asset relative to the
// document base, the way a lazy route chunk does.
const base = window.__BIFROST_BASE__;
const link = document.createElement("link");
link.rel = "preload";
link.as = "script";
link.href = new URL("_nuxt/route.D4kq8z.js", document.baseURI).href;
document.head.append(link);
document.getElementById("__nuxt").dataset.base = base;
