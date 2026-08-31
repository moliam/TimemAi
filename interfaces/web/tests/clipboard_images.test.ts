import { describe, expect, it } from "vitest";
import { clipboardImageFiles } from "../src/clipboard_images";

describe("clipboard images", () => {
  it("selects only images and names unnamed clipboard images", () => {
    const image = new File(["png"], "image.png", { type: "image/png", lastModified: 7 });
    const text = new File(["text"], "note.txt", { type: "text/plain" });
    const files = clipboardImageFiles([
      { kind: "file", type: "image/png", getAsFile: () => image },
      { kind: "file", type: "text/plain", getAsFile: () => text },
    ], 1234);
    expect(files).toHaveLength(1);
    expect(files[0].name).toBe("pasted-image-1234.png");
    expect(files[0].type).toBe("image/png");
  });

  it("keeps useful original image names", () => {
    const image = new File(["jpg"], "diagram.jpg", { type: "image/jpeg" });
    expect(clipboardImageFiles([
      { kind: "file", type: "image/jpeg", getAsFile: () => image },
    ])[0]).toBe(image);
  });
});
