// Pair user prompts with saved images. Failures stay text.
function imagineBoard(items) {
  const pattern = /\.blora\/images\/([A-Za-z0-9_-]+)\/([a-fA-F0-9]{64}\.(?:png|jpe?g|webp))/g;
  let prompt = "";
  const cards = [];
  const errors = [];
  for (const item of items || []) {
    const kind = item.kind || "";
    const text = item.text || "";
    if (kind === "user") {
      prompt = (text.split("\n")[0] || "").trim();
      continue;
    }
    if (kind !== "tools" && kind !== "tool") {
      continue;
    }
    const found = [];
    pattern.lastIndex = 0;
    let match = pattern.exec(text);
    while (match) {
      found.push({ session: match[1], name: match[2], path: match[0] });
      match = pattern.exec(text);
    }
    if (found.length) {
      for (const image of found) {
        cards.push({ prompt, path: image.path, session: image.session, name: image.name });
      }
      continue;
    }
    const failed = /失败|error|denied|HTTP|provider|没有密钥|not found/i.test(text);
    if (failed && text.trim()) {
      errors.push(text.trim());
    }
  }
  return { cards, errors };
}

if (typeof module !== "undefined") {
  module.exports = { imagineBoard };
}
