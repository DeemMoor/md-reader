# Разработка MD Reader

## Сборка и тесты

```sh
cargo build --release              # target/release/mdreader(.exe)
cargo test
node --test tests/diff.test.js     # движок подсветки изменений
```

На Linux нужны `libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev`.

## Установщики

Собираются [cargo-packager](https://github.com/crabnebula-dev/cargo-packager), результат — в `target/dist`:

```sh
cargo install cargo-packager --locked
cargo packager --release --formats wix,nsis       # Windows: .msi и setup.exe
cargo packager --release --formats dmg            # macOS
cargo packager --release --formats deb,appimage   # Linux
```

## Устройство

- `src/main.rs` — окно (tao), меню (muda), события, сессия
- `src/server.rs` — протокол `mdr://`: оболочка, документы, ресурсы, список файлов
- `src/render.rs` — Markdown → HTML (pulldown-cmark), подсветка кода (syntect)
- `src/settings.rs`, `src/instance.rs`, `src/platform.rs` — настройки, единственный экземпляр, особенности ОС
- `src/update.rs` — проверка обновлений и загрузка установщика с проверкой SHA-256
- `src/log.rs` — журнал ошибок и черновик отчёта о проблеме
- `src/clipboard.rs` — «Создать из буфера»: чтение буфера обмена, HTML → Markdown (htmd)
- `assets/diff.js` — подсветка изменений: алгоритм Майерса по блокам, строкам и словам
- `assets/` — интерфейс (`shell.*`), стили и скрипт документа (`doc.*`), встроенные KaTeX и Mermaid (MIT, `assets/vendor`)
- `packaging/nsis/installer.nsi` — шаблон NSIS с исправлением снятия ассоциаций при удалении

## Выпуск версии

1. Каждое заметное для пользователя изменение записывается в `CHANGELOG.md` в раздел «Не выпущено» тем же коммитом, что и код.
2. Перед выпуском раздел переименовывается в `## X.Y.Z — ГГГГ-ММ-ДД`, а в `Cargo.toml` поднимается `version`.
3. Коммит и тег:

   ```sh
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

GitHub Actions соберёт установщики для трёх ОС, посчитает `SHA256SUMS.txt` и опубликует релиз с описанием из раздела `CHANGELOG.md`. Сборка остановится, если тег не совпадает с версией в `Cargo.toml` или в `CHANGELOG.md` нет раздела для этой версии. По этому описанию программа показывает пользователям, что нового.
