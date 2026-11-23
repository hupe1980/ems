// The household page's service worker. One job: show a notification the box
// pushed, in browsers that do not show a Declarative Web Push message on their
// own (Safari does, and never runs this), and open the page when it is tapped.
// It handles no requests and keeps nothing: the page works without it.

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

self.addEventListener("push", (event) => {
  let message = null;
  try { message = event.data.json(); } catch { /* not ours */ }
  const n = message && message.web_push === 8030 && message.notification;
  // A push must end in a visible notification or the browser withdraws the
  // subscription, so a message this worker cannot read still says something.
  const title = (n && n.title) || "hems";
  event.waitUntil(self.registration.showNotification(title, {
    body: (n && n.body) || "",
    lang: (n && n.lang) || "de",
    dir: (n && n.dir) || "auto",
    tag: (n && n.tag) || "hems",
    icon: "/icon.svg",
    data: { navigate: (n && n.navigate) || "/", event_id: n?.data?.event_id },
  }));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  // Only ever this origin: the box wrote the address, and a worker that opened
  // whatever it was handed would be a way to open anything.
  const target = new URL(event.notification.data.navigate, self.location.origin);
  const url = target.origin === self.location.origin ? target.href : self.location.origin + "/";
  event.waitUntil((async () => {
    const open = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    for (const client of open) {
      if (new URL(client.url).origin === self.location.origin) {
        await client.focus();
        if ("navigate" in client) await client.navigate(url).catch(() => {});
        return;
      }
    }
    await self.clients.openWindow(url);
  })());
});
