# PKG Peek

[![CI](https://github.com/m4urigonz/pkg_peek/actions/workflows/ci.yml/badge.svg)](https://github.com/m4urigonz/pkg_peek/actions/workflows/ci.yml)

*Windows Explorer shell extension for PS4 PKG files: game icon in every view, rich preview in Explorer's preview pane and PowerToys Peek, and Title ID / version / firmware columns. UI in Spanish.*

Integración de los **PKG de PS4** en el Explorador de Windows:

- **Icono del juego** en todas las vistas: Detalles, Lista, Iconos pequeños/grandes, Mosaicos.
- **Insignia** en las miniaturas: `UPD 1.11` para actualizaciones, `DLC` para contenido adicional y `!` si la descarga está incompleta.
- **Vista previa** en el panel del Explorador (`Alt+P`) y en PowerToys Peek (`Ctrl+Espacio`): portada, título, Title ID, Content ID, versión, firmware requerido, región, tamaño, fecha y notas del parche. Con un clic en el Title ID o el Content ID se copian; con clic derecho aparecen más opciones.
- **Metadatos**: tooltip, columnas en la vista Detalles (*Title ID*, *Tipo de PKG*, *Versión app*, *Firmware requerido*…), pestaña Detalles de Propiedades y búsqueda de Windows por Title ID.
- **Doble clic**: abre el visor `pkgpeek.exe`. Su menú contextual permite guardar el icono o el fondo y mostrar el archivo en su carpeta.

Los updates que no traen icono propio usan el del juego base si está en la misma carpeta, y si no, el icono del paquete de trofeos. Los `.pkg` que no son de PS4 (PS3, macOS) muestran un icono genérico.

## Instalación

1. Descarga `PkgPeek-Setup-x.y.z.exe` de [Releases](https://github.com/m4urigonz/pkg_peek/releases/latest).
2. Ejecútalo y acepta el aviso de administrador. Hace falta porque el property handler y el esquema de propiedades se registran para toda la máquina.
3. Si quieres ver los iconos al momento, marca *Reiniciar el Explorador de Windows al terminar*.

El instalador no está firmado, así que Windows puede mostrar "Windows protegió su PC": pulsa *Más información → Ejecutar de todas formas*. Requiere Windows 10/11 de 64 bits.

Para actualizar basta con instalar la versión nueva encima. Para desinstalar: *Configuración → Aplicaciones → Aplicaciones instaladas → PKG Peek*.

Instalación silenciosa (para scripts o winget):

```powershell
PkgPeek-Setup-0.1.0.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
"C:\Program Files\PkgPeek\unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

### Desde el código

Requiere Rust (toolchain MSVC) y las Build Tools de Visual Studio con el Windows SDK.

```powershell
.\scripts\install.ps1                  # compila, pide UAC, copia a C:\Program Files\PkgPeek y registra
.\scripts\install.ps1 -RestartExplorer # además reinicia el Explorador y limpia la caché de iconos
.\scripts\uninstall.ps1                # quita el registro y los archivos
.\scripts\build-installer.ps1          # genera dist\PkgPeek-Setup-<versión>.exe (necesita Inno Setup 6)
```

## Limitaciones conocidas

- **Archivos descargados de Internet**: Windows no muestra la vista previa en el panel de los archivos con la marca "descargado de Internet" (*Mark of the Web*) y enseña un aviso en su lugar. Los iconos, las miniaturas y las columnas sí funcionan. Para quitar la marca: Propiedades → *Desbloquear*, o `Get-ChildItem C:\ruta\a\tus\pkg -Filter *.pkg | Unblock-File`.
- En el panel de vista previa, Windows solo le pasa al handler el contenido del archivo, sin su ruta. Por eso un update sin icono propio muestra ahí el icono de trofeos en vez de la portada del juego base. La miniatura y el visor sí usan la del juego base.
- El Explorador guarda en caché miniaturas e iconos. Si un icono no se actualiza, usa `install.ps1 -RestartExplorer`.

## Estructura

| Crate | Qué hace |
|---|---|
| `crates/pkg-core` | Lectura de PKG de PS4, PARAM.SFO, trofeos y búsqueda del juego base (Rust puro) |
| `crates/pkg-render` | Miniaturas y ficha con WIC, Direct2D y DirectWrite; ventana de la ficha |
| `crates/pkgpeek-shell` | `pkgpeek_shell.dll`: servidor COM con los handlers de miniatura, icono, vista previa y propiedades |
| `crates/pkgpeek` | `pkgpeek.exe` (visor) y `pkgpeek-cli.exe` (herramientas de prueba) |

## Desarrollo

```powershell
cargo test --workspace
$env:PKGPEEK_SAMPLES='C:\ruta\a\tus\pkg'; cargo test -p pkg-core --test samples -- --nocapture

pkgpeek-cli info  juego.pkg [--json]
pkgpeek-cli thumb juego.pkg thumb.png --size 256
pkgpeek-cli card  juego.pkg card.png --width 420 --height 720
pkgpeek-cli shelltest target\debug\pkgpeek_shell.dll juego.pkg out\   # prueba los handlers COM sin registrarlos
pkgpeek-cli shellcheck juego.pkg out\                                 # consulta al shell ya instalado
```

**Publicar una versión**: sube `version` en `[workspace.package]` de `Cargo.toml` (los ejecutables y el instalador la toman de ahí), haz commit y sube un tag `vX.Y.Z` con esa misma versión. El workflow `release.yml` compila, genera el instalador y publica la release en GitHub.

```powershell
git tag v0.2.0
git push origin v0.2.0
```

**Diagnóstico**: si existe la carpeta `%USERPROFILE%\AppData\LocalLow\PkgPeek`, la extensión escribe ahí un `debug.log`. Se usa LocalLow porque es donde puede escribir el `prevhost.exe` de integridad baja que aloja la vista previa. Borra la carpeta para desactivar el log.

**Nota técnica**: los objetos COM hechos con `#[implement]` de windows-rs son *agile* por defecto (free-threaded marshaler). En un preview handler eso hace que `prevhost` llame a `DoPreview` desde un hilo worker de RPC: la ventana se crea en un hilo sin bucle de mensajes, nunca se pinta y cuelga el Explorador. Por eso todos los handlers usan `Agile = false`.
