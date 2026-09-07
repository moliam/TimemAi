# Web 界面多语言（i18n）架构

本文定义 `interfaces/web` 的界面文案多语言架构。范围仅限 Interface 呈现层：
Core、Bridge、Shell、协议与线格式不涉及；语言偏好是浏览器本地状态，不进入
Host 权威状态，不改变任何命令、事件或会话语义。

## 1. 分层与职责

```text
src/i18n/
  locale.ts        # 语言状态：读取/写入/订阅/持久化，<html lang> 同步
  strings.zh.ts    # 中文目录（源目录），导出 Strings / StringKey 类型
  strings.en.ts    # 英文目录，显式标注 Strings 类型 → 键集与 zh 编译期强制齐平
  index.ts         # t(key, params?) 取词函数与 useT() 订阅入口
```

- `locale.ts` 与 `appearance.ts`、`beta_preferences.ts` 同构：localStorage 持久化
  （键 `timem-web-locale-v1`，存裸值 `"zh" | "en"`，不是 JSON）、默认跟随
  `navigator.languages`、存储被禁用时降级为 tab 内选择、storage 事件跨标签同步。
- 目录只承载 Interface 呈现文本。模型输出、Host 权威状态投影、Session/Role 等
  用户数据、协议字符串、审计与日志永不翻译。

## 2. 硬性规则

1. **JSX 与 TS 源码中禁止内联用户可见文案**。按钮文本、placeholder、
   aria-label、title、错误叙述一律走 `t("domain.key", params)`；
   代码注释不受影响。
2. **CSS `content` 伪元素文案必须数据化**：写成 `content: attr(data-collapsed-label)`
   等形式，由 React 把目录值放到伪元素的宿主元素（通常是 `<summary>`）的
   `data-*` 属性上。`attr()` 只能读取伪元素宿主自身的属性，不能读祖先节点。
3. **键按功能域分组**（`domain.key`），插值占位符使用 `{name}` 形式，仅支持
   字符串/数字替换；复数等语法化需求出现时，先扩展 `interpolate` 并补充测试，
   不得在调用点手写英文复数分支。
4. **React 订阅规则**：渲染 `t()` 输出的组件必须通过 `useT()`（或位于已订阅
   祖先之下）订阅语言变化；`memo` 组件跨越了父级重渲染路径，必须自行调用
   `useT()`。禁止在模块作用域缓存 `t()` 输出——语言切换不会刷新它。
5. **非 hook 场景**（view-model 派生函数、事件回调）可以直接调用 `t()`，但结果
   必须在订阅组件的渲染期被消费，而不是在事件发生时缓存进长期状态。
6. 目录值不得为空字符串；两个目录的键集必须完全一致（`Strings` 类型 +
   `i18n.test.ts` 双重强制）。

## 3. 守卫与测试

- `tests/i18n_source_guard.test.ts`：剥离注释后扫描 `src/`（i18n/ 除外），
  禁止 CJK 字符字面量，防止内联中文文案回流。
- `tests/i18n.test.ts`：键集齐平、值非空、插值、运行时切换、未知键回退。
- 断言目录文案语义的测试必须显式 `setLocale("zh")`（vitest jsdom 的
  `navigator.languages` 是 en-US，不固定语言的断言会漂移）；优先断言
  `zh.*` 目录值 + 源码 `t("key")` 引用，而不是渲染产物。
- 浏览器验收（`tests/browser/*.mjs`）以 `--lang=zh-CN --accept-lang=zh-CN`
  启动 Chrome，保证默认 locale 与既有中文断言一致。

## 4. 接入新语言

1. 复制 `strings.en.ts` 为 `strings.<tag>.ts`，实现 `Strings` 类型（缺键/多键
   都是编译错误）。
2. 在 `index.ts` 的 `catalogs` 注册；在 `locale.ts` 扩展 `Locale` 类型、
   `browserLocale()` 的语言映射与 `applyDocumentLanguage()` 的 BCP-47 标签。
3. 在设置中心「界面语言」分段控件中加入选项。
4. 在 `i18n.test.ts` 的目录校验循环中加入新目录。

## 5. 覆盖范围与已知边界

本轮已迁移 Web 全部界面 chrome 文案（会话/输入区/待发送队列/Role 库/聊天检索库/
设置中心/接入点/收藏空间/MCP/工具与重试状态/错误叙述/重启目录门）。工具状态的
两个自造词（后台运行、已超时）走目录；未知状态按原始 wire 值显示，不做猜测性
翻译。来自 Host 的业务叙述文本（如审批决定、决策原因）随 Host 投影到达，其
语言策略属于 Host/Core 侧，不在本模块范围。
