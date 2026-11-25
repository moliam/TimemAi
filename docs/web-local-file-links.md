# WebUI 本地文件链接的远程预览

## 目标

模型最终回答中的本地文件链接在远程 WebUI 里原本只能指向 `file:///...` 或本机绝对路径，远端浏览器无法打开。现在 WebUI 会识别这些链接，用户点击时向 Host 换取一个短期票据，并在新标签页打开只读预览。

## 识别范围

- `file:///absolute/path`（含 URL 编码、Windows 盘符路径）
- 当前 Session CWD 下的绝对路径
- `./`、`../` 开头或带扩展名/目录分隔符的相对路径
- 带 `#L10` 等片段或 URL 编码的路径会先归一化为文件路径

普通站内路由（如 `/docs`、`/settings/...`）、`http(s)`、`mailto`、锚点和协议相对 URL 不会被误判为本地文件。

## 交互

1. Markdown 链接渲染为本地文件链接，点击不会直接导航到本机路径。
2. 前端先打开空白新标签页并显示“正在创建临时预览”。
3. 前端调用 `POST /api/file-preview-links`，携带 Session ID 与归一化路径。
4. Host 校验路径后生成 128-bit 随机票据，返回 `/api/file-preview/<ticket>`。
5. 新标签页导航到该地址，Host 返回只读 HTML 预览页。

## 安全边界

- 票据只存在内存中，10 分钟 TTL，最多 128 个；过期/超限自动淘汰。
- 预览 URL 不携带长期 WebUI access token，票据本身是短期 capability。
- 只允许当前 Session CWD 和配置的 workspace 根目录内的常规文件；目标路径在签发与访问时都会 `canonicalize`，拒绝 `..`、绝对路径越界和符号链接逃逸。
- HTML/XML/SVG 等内容按文本转义显示，不作为活动页面执行；只有常见位图扩展名会以图片响应。
- 预览页使用严格 CSP、`no-store`、`no-referrer`、`nosniff`，不执行脚本。
- 文本预览最多 1 MiB，图片最多 16 MiB，下载最多 64 MiB；超限时只显示截断提示或拒绝下载。
- 下载以 `application/octet-stream` + attachment 返回，文件名做 ASCII 安全化。

## 测试看护

- Rust `file_preview` 单测：canonical root 边界、Ticket TTL/容量、图片类型、下载文件名。
- Host 集成测试：票据签发、路径逃逸拒绝、文本预览的 HTML 转义、公共模式未认证请求 401。
- Bridge 路由测试：`POST /api/file-preview-links` 与 `GET /api/file-preview/:ticket` 的方法约束。
- Web 单测：`file_links` 路径识别、Markdown provider 下本地链接渲染、i18n 与安全 URL 回归。

## 当前限制

模型若链接到 Session CWD/配置 workspace 之外的文件（例如 `/tmp/...`），预览会明确失败而不是放宽边界。这样远程 WebUI 不会因为一条 Markdown 链接获得任意本机文件读取能力。
