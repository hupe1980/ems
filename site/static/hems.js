(function () {
  "use strict";

  var root = document.documentElement;
  var LABEL = { light: "☀", dark: "☾", system: "◐" };
  var ORDER = ["system", "light", "dark"];

  function apply(choice) {
    if (choice === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", choice);

    // Syntax highlighting follows the selected theme as well.
    var light = document.getElementById("syntax-light");
    var dark = document.getElementById("syntax-dark");
    if (light && dark) {
      if (choice === "system") {
        light.media = "(prefers-color-scheme: light)";
        dark.media = "(prefers-color-scheme: dark)";
      } else {
        light.media = choice === "light" ? "all" : "not all";
        dark.media = choice === "dark" ? "all" : "not all";
      }
    }
    var btn = document.getElementById("theme-toggle");
    if (btn) {
      btn.textContent = LABEL[choice];
      btn.setAttribute("aria-label", "Colour theme: " + choice);
      btn.title = "Colour theme: " + choice;
    }
    document.dispatchEvent(new CustomEvent("hems:theme", { detail: choice }));
  }

  function stored() {
    try {
      var saved = localStorage.getItem("hems-theme");
      return ORDER.includes(saved) ? saved : "system";
    } catch { return "system"; }
  }

  var current = stored();
  apply(current);

  var toggle = document.getElementById("theme-toggle");
  if (toggle) {
    toggle.hidden = false;
    toggle.addEventListener("click", function () {
      current = ORDER[(ORDER.indexOf(current) + 1) % ORDER.length];
      try { localStorage.setItem("hems-theme", current); } catch (e) { /* private mode */ }
      apply(current);
    });
  }

  document.querySelectorAll("article table").forEach(function (table) {
    if (table.parentElement.classList.contains("table-wrap")) return;
    var wrap = document.createElement("div");
    wrap.className = "table-wrap";
    table.parentNode.insertBefore(wrap, table);
    wrap.appendChild(table);
  });

  if (navigator.clipboard) {
    document.querySelectorAll("pre:not(.mermaid) > code").forEach(function (code) {
      var pre = code.parentElement;
      var btn = document.createElement("button");
      btn.className = "copy-btn";
      btn.type = "button";
      btn.textContent = "Copy";
      btn.addEventListener("click", function () {
        var text = code.textContent.replace(/^\$ /gm, "");
        navigator.clipboard.writeText(text).then(function () {
          btn.textContent = "Copied";
        }, function () {
          btn.textContent = "Copy unavailable";
        }).finally(function () {
          setTimeout(function () { btn.textContent = "Copy"; }, 1400);
        });
      });
      pre.appendChild(btn);
    });
  }

  const box = document.getElementById("search");
  if (box) {
    const input = document.getElementById("search-input");
    const list = document.getElementById("search-results");
    const status = document.getElementById("search-status");
    let pendingIndex;

    function loadIndex() {
      if (!pendingIndex) {
        const library = new Promise((resolve, reject) => {
          if (window.elasticlunr) return resolve();
          const script = document.createElement("script");
          script.src = box.dataset.library;
          script.onload = resolve;
          script.onerror = reject;
          document.head.appendChild(script);
        });
        const data = fetch(box.dataset.index).then(response => {
          if (!response.ok) throw new Error("Search index unavailable");
          return response.json();
        });
        pendingIndex = Promise.all([library, data])
          .then(([, raw]) => window.elasticlunr.Index.load(raw))
          .catch(error => {
            pendingIndex = undefined;
            throw error;
          });
      }
      return pendingIndex;
    }

    function clearResults() {
      list.replaceChildren();
      status.textContent = "";
    }

    function message(text) {
      const item = document.createElement("li");
      item.className = "search-empty";
      item.textContent = text;
      list.replaceChildren(item);
      status.textContent = text;
    }

    async function search() {
      const term = input.value.trim();
      clearResults();
      if (term.length < 2) return;
      try {
        const index = await loadIndex();
        // Focus and the query can both change while the index downloads.
        if (input.value.trim() !== term || !box.contains(document.activeElement)) return;
        const hits = index.search(term, {
          bool: "AND", expand: true,
          fields: { title: { boost: 3 }, description: { boost: 2 }, body: { boost: 1 } }
        });
        if (!hits.length) return message("No matching guides");

        const results = hits.slice(0, 8).map(hit => {
          const doc = index.documentStore.getDoc(hit.ref);
          const link = document.createElement("a");
          link.href = hit.ref;
          const title = document.createElement("strong");
          title.textContent = doc.title;
          const description = document.createElement("span");
          description.textContent = doc.description;
          link.append(title, description);
          const item = document.createElement("li");
          item.appendChild(link);
          return item;
        });
        list.replaceChildren(...results);
        status.textContent = hits.length > results.length
          ? "Showing the first " + results.length + " of " + hits.length + " results"
          : hits.length + (hits.length === 1 ? " result" : " results");
      } catch {
        if (input.value.trim() === term && box.contains(document.activeElement)) {
          message("Search unavailable. Browse the guides below.");
        }
      }
    }

    input.addEventListener("focus", search);
    input.addEventListener("input", search);
    box.addEventListener("focusout", event => {
      if (!box.contains(event.relatedTarget)) clearResults();
    });
    box.addEventListener("keydown", event => {
      if (event.key === "Escape") {
        event.preventDefault();
        input.value = "";
        input.focus();
        clearResults();
      }
      if (event.key === "ArrowDown" && event.target === input) {
        const first = list.querySelector("a");
        if (first) {
          event.preventDefault();
          first.focus();
        }
      }
    });
    box.hidden = false;
  }

  document.addEventListener("keydown", function (e) {
    if (e.key !== "/" || e.ctrlKey || e.metaKey || e.altKey) return;
    var t = e.target;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
    var input = document.getElementById("search-input");
    if (input && !document.getElementById("search").hidden) { e.preventDefault(); input.focus(); }
  });

  const toc = document.querySelector(".toc details");
  if (toc) {
    const compact = window.matchMedia("(max-width: 75rem)");
    toc.open = !compact.matches;
    compact.addEventListener("change", event => { toc.open = !event.matches; });
  }

  var tocLinks = document.querySelectorAll("aside.toc a[href*='#']");
  if (tocLinks.length && "IntersectionObserver" in window) {
    var byId = {};
    tocLinks.forEach(function (a) { byId[decodeURIComponent(a.hash.slice(1))] = a; });
    var heads = Array.prototype.filter.call(
      document.querySelectorAll("article h2[id], article h3[id]"),
      function (h) { return byId[h.id]; });
    var activeHeading = null;
    var spy = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (!en.isIntersecting) return;
        if (activeHeading) {
          activeHeading.classList.remove("active");
          activeHeading.removeAttribute("aria-current");
        }
        activeHeading = byId[en.target.id];
        activeHeading.classList.add("active");
        activeHeading.setAttribute("aria-current", "location");
      });
    }, { rootMargin: "0px 0px -75% 0px" });
    heads.forEach(function (h) { spy.observe(h); });
  }

  const diagrams = Array.from(document.querySelectorAll("pre.mermaid"));
  if (!diagrams.length) return;
  const sources = diagrams.map(element => {
    const copy = element.cloneNode(true);
    copy.querySelectorAll("br").forEach(br => br.replaceWith("<br/>"));
    return copy.textContent;
  });

  // The source remains readable offline or when a graph fails to render.
  import("https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.esm.min.mjs").then(({ default: mermaid }) => {
    const system = window.matchMedia("(prefers-color-scheme: dark)");
    let revision = 0;
    async function render() {
      const version = ++revision;
      const dark = root.dataset.theme ? root.dataset.theme === "dark" : system.matches;
      mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: dark ? "dark" : "neutral" });
      for (const [i, element] of diagrams.entries()) {
        if (version !== revision) return;
        try {
          const { svg } = await mermaid.render("diagram-" + version + "-" + i, sources[i]);
          if (version !== revision) return;
          element.innerHTML = svg;
          element.dataset.processed = "true";
        } catch {
          if (version !== revision) return;
          element.textContent = sources[i];
          delete element.dataset.processed;
        }
      }
    }
    render();
    document.addEventListener("hems:theme", render);
    system.addEventListener("change", render);
  }).catch(() => {});
})();
