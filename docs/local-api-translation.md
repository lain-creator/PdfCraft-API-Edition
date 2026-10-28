# PDFCraft API 翻译定制版

基于上游稳定版本 v0.14.0；非上游官方发行版。不会改写原 PDF。

`0.14.0-api.20261029.1`：编译内置 798 条 PCB 公共词库，干净安装无需导入；本机补充独立保留。详见 [公共词库](local-public-pcb-glossary.md)。

## 设置与使用

1. 点击顶部「API 设置」，填写服务商基础地址、文字模型 ID、API Key。窄窗口下，翻译按钮集中在顶部「API 翻译」菜单，不挤占原来的页面区域。可选择 GPT / OpenAI、MiniMax、DeepSeek、Kimi、小米 MiMo 或自定义兼容 API；协议支持 Chat Completions / Responses。预设不代表账号权限或所有厂商专有协议兼容。
2. 文字 PDF：选择「文字选区」，在同一页拖动选中文字，点击「中译英」或「英译中」。仅发送选中的文字和命中术语，不上传 PDF 文件。
3. 扫描件/图片：选择「图片框选」，拖出页面区域，确认弹窗中的裁剪预览后点击翻译方向。先调用视觉模型识别，再调用文字模型翻译，**共两次 API，可能分别计费**。需要配置确实支持图片输入的视觉模型 ID，不能用仅文字模型代替。
4. 原文/识别文字和译文均在可缩放、折叠的窗口里显示，可分别复制。不会创建 PDF 编辑、撤销记录或自动保存文档。切换文档或关闭弹窗会取消本机等待；已经发送的请求可能继续在服务端处理。

文字选区最多 64 KB，超过上限明确拒绝、不截断；暂不支持跨页全选翻译。图片限定同一页、长边最多 2048 像素/最高 144 dpi/约 4 百万像素/8 MB PNG。小字或密集表格请缩小框选区域；识别错误和译文仍需人工核对。遵守 PDF 的复制权限。对图片遮挡、复杂公式、模糊扫描件不承诺完美识别。

## 词库与隐私

本机 PCB 词库按最长词组和英文边界匹配，多义词结合语境，已知错误词对隔离。不连接旧翻译网站。图片识别之后，只把识别原文命中的术语发送给文字模型。保留压合/lamination、芯板/core、叠层/stack-up 的行业提示，并检查数字改变、明显的压合/press-fit 误译和无来源附件声明；这些检查不能代替专业审校。

公开包内置 `pcb-glossary-public.json`。个人词库缓存为用户配置目录内的 `pcb-glossary.json`，不会公开分发。默认位置：Mac `~/Library/Application Support/PdfCraftAPI/`；Windows `%APPDATA%/PdfCraftAPI/`。无需手动导入公共词库，原个人文件保留。

API Key 默认仅本次运行，勾选记住后存放于系统密钥存储；普通 JSON 不保存密钥。默认只允许 HTTPS（本机 HTTP 例外），远程 HTTP 必须明确允许；不跟随重定向，不自动重试。服务商可看到发送的内容，数据保留规则以该服务商为准。

## 自动化

`api_selection_text` 准备真实 glyph 范围，`api_selection_image` 生成裁剪预览，均不联网。`api_translate` 每次必须显式传 `allow_api_send:true`，用传入 profile 和 `api_key_env`；不会自动读取桌面的密钥或词库。可选 `glossary_path` 受 `--root` 限制。详见 CLI tools。

目前验证使用合成 PDF 和本机模拟 API，不使用业务文档，也不调用真实付费模型。Mac 版本临时签名、未公证。独立公开仓库为 [PdfCraft-API-Edition](https://github.com/lain-creator/PdfCraft-API-Edition)，安装包见该仓库 Releases；本版首发 Mac arm64 包，不把上游其他平台能力当作已发布的安装包。

本机 arm64 打包使用 `cargo xtask local-api-package`，保留上游离线 OCR 模型和许可。下载不可用时可设置 `PDFCRAFT_MODELS_DIR` 指向模型缓存；模型和许可必须逐个匹配 `ATTRIBUTION.toml` 的固定 SHA-256，否则拒绝打包。不拷贝旧安装中的其他文件或个人设置。

Mac Intel CI 可设置 `PDFCRAFT_PACKAGE_BINARY` 指向交叉构建的 arm64 可执行文件；打包前检查 Mach-O 架构，避免错标平台。
