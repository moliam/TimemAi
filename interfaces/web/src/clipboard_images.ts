export type ClipboardFileItem = Pick<DataTransferItem, "kind" | "type" | "getAsFile">;

function extensionForImageType(type: string) {
  const subtype = type.slice("image/".length).toLowerCase();
  if (subtype === "jpeg") return "jpg";
  const extension = subtype.replace("+xml", "");
  return /^[a-z0-9.-]+$/.test(extension) && extension ? extension : "png";
}

export function clipboardImageFiles(items: readonly ClipboardFileItem[], now = Date.now()): File[] {
  let imageIndex = 0;
  return items.flatMap((item) => {
    if (item.kind !== "file" || !item.type.toLowerCase().startsWith("image/")) return [];
    const file = item.getAsFile();
    if (!file) return [];
    imageIndex += 1;
    const usefulName = file.name.trim() && !/^image\.(png|jpe?g|gif|webp)$/i.test(file.name);
    if (usefulName) return [file];
    const suffix = imageIndex === 1 ? "" : `-${imageIndex}`;
    return [new File(
      [file],
      `pasted-image-${now}${suffix}.${extensionForImageType(file.type || item.type)}`,
      { type: file.type || item.type, lastModified: file.lastModified },
    )];
  });
}
