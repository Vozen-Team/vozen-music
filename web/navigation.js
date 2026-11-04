function renderAccount(user) {
  const link = $("navLogin");
  link.replaceChildren();
  link.classList.toggle("nav__login--account", !!user);
  if (!user) {
    bindMusicText(link, t("Log in"));
    bindMusicText(link, null, "aria-label");
    return;
  }
  const name = user.global_name || user.username;
  bindMusicText(link, t("Discord account: {name}", { name }), "aria-label");
  const avatar = document.createElement("span");
  avatar.className = "discord-avatar nav__login-av-wrap";
  const image = document.createElement("img");
  image.className = "nav__login-av";
  image.width = 24;
  image.height = 24;
  bindMusicText(image, t("Avatar for {name}", { name }), "alt");
  image.referrerPolicy = "no-referrer";
  const initial = document.createElement("span");
  initial.className = "nav__login-av nav__login-av--none";
  initial.textContent = name.slice(0, 1).toUpperCase();
  if (
    /^\d{16,22}$/.test(user.id) &&
    /^(?:a_)?[a-f0-9]{32}$/.test(user.avatar || "")
  ) {
    image.src = `https://cdn.discordapp.com/avatars/${user.id}/${user.avatar}.webp?size=32`;
    image.addEventListener("error", () => image.replaceWith(initial), {
      once: true,
    });
    avatar.append(image);
  } else avatar.append(initial);
  const asset = user.avatar_decoration_data?.asset;
  if (/^[A-Za-z0-9_]{1,128}$/.test(asset || "")) {
    const decoration = document.createElement("img");
    decoration.className = "discord-avatar__decoration";
    decoration.alt = "";
    decoration.setAttribute("aria-hidden", "true");
    decoration.width = 256;
    decoration.height = 256;
    decoration.referrerPolicy = "no-referrer";
    decoration.src = `https://cdn.discordapp.com/avatar-decoration-presets/${asset}.png?size=256`;
    decoration.addEventListener("error", () => decoration.remove(), {
      once: true,
    });
    avatar.append(decoration);
  }
  const label = document.createElement("span");
  label.textContent = name;
  link.append(avatar, label);
}

function closeNavigation() {
  $("vozen-primary-links").classList.remove("is-open");
  $("burger").setAttribute("aria-expanded", "false");
  $("langMenu").classList.remove("is-open");
  $("langBtn").setAttribute("aria-expanded", "false");
}
$("burger").addEventListener("click", () => {
  const open = $("burger").getAttribute("aria-expanded") !== "true";
  closeNavigation();
  $("vozen-primary-links").classList.toggle("is-open", open);
  $("burger").setAttribute("aria-expanded", String(open));
});
$("langBtn").addEventListener("click", () => {
  const open = $("langBtn").getAttribute("aria-expanded") !== "true";
  closeNavigation();
  $("langMenu").classList.toggle("is-open", open);
  $("langBtn").setAttribute("aria-expanded", String(open));
  if (open) $("langPanel").querySelector("a")?.focus();
});
$("langPanel").addEventListener("keydown", (event) => {
  const options = [...$("langPanel").querySelectorAll("a")];
  const index = options.indexOf(document.activeElement);
  let next;
  if (event.key === "ArrowDown") next = (index + 1) % options.length;
  else if (event.key === "ArrowUp")
    next = (index + options.length - 1) % options.length;
  else if (event.key === "Home") next = 0;
  else if (event.key === "End") next = options.length - 1;
  if (next !== undefined) {
    event.preventDefault();
    options[next].focus();
  }
});
document.addEventListener("keydown", (event) => {
  if (
    event.key !== "Escape" ||
    ($("langBtn").getAttribute("aria-expanded") !== "true" &&
      $("burger").getAttribute("aria-expanded") !== "true")
  )
    return;
  const button =
    $("langBtn").getAttribute("aria-expanded") === "true"
      ? $("langBtn")
      : $("burger");
  closeNavigation();
  button.focus();
});
document.addEventListener("click", (event) => {
  if (!$("nav").contains(event.target)) closeNavigation();
});
window.matchMedia("(min-width:1280px)").addEventListener("change", (event) => {
  if (event.matches) closeNavigation();
});
window.addEventListener(
  "scroll",
  () => $("nav").classList.toggle("is-stuck", scrollY > 12),
  { passive: true },
);
