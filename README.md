# PKG Peek

Integración de los **PKG de PS4** en el Explorador de Windows:

- **Icono del juego** en todas las vistas: Detalles, Lista, Iconos pequeños/grandes, Mosaicos.
- **Insignia** en las miniaturas: `UPD 1.11` para actualizaciones, `DLC` para contenido adicional y `!` si la descarga está incompleta.
- **Vista previa** en el panel del Explorador (`Alt+P`) y en PowerToys Peek (`Ctrl+Espacio`): portada, título, Title ID, Content ID, versión, firmware requerido, región, tamaño, fecha y notas del parche. Con un clic en el Title ID o el Content ID se copian; con clic derecho aparecen más opciones.
- **Metadatos**: tooltip, columnas en la vista Detalles (*Title ID*, *Tipo de PKG*, *Versión app*, *Firmware requerido*…), pestaña Detalles de Propiedades y búsqueda de Windows por Title ID.
- **Doble clic**: abre el visor `pkgpeek.exe`. Su menú contextual permite guardar el icono o el fondo y mostrar el archivo en su carpeta.

Los updates que no traen icono propio usan el del juego base si está en la misma carpeta, y si no, el icono del paquete de trofeos. Los `.pkg` que no son de PS4 (PS3, macOS) muestran un icono genérico.

## Instalar y desinstalar

Requiere Rust (toolchain MSVC) y las Build Tools de Visual Studio con el Windows SDK.

```powershell
.\scripts\install.ps1                 # compila, pide UAC, copia a C:\Program Files\PkgPeek y registra
.\scripts\install.ps1 -RestartExplorer  # además reinicia el Explorador y limpia la caché de iconos
.\scripts\uninstall.ps1               # quita el registro y los archivos
```

Hace falta ser administrador porque el property handler y el esquema de propiedades se registran para toda la máquina (HKLM). Para actualizar basta con volver a ejecutar `install.ps1`.

## Limitaciones conocidas

- **Archivos descargados de Internet**: Windows no muestra la vista previa en el panel de los archivos con la marca "descargado de Internet" (*Mark of the Web*) y enseña un aviso en su lugar. Los iconos, las miniaturas y las columnas sí funcionan. Para quitar la marca: Propiedades → *Desbloquear*, o `Get-ChildItem D:\Descargas -Filter *.pkg | Unblock-File`.
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
$env:PKGPEEK_SAMPLES='D:\Descargas'; cargo test -p pkg-core --test samples -- --nocapture

pkgpeek-cli info  juego.pkg [--json]
pkgpeek-cli thumb juego.pkg thumb.png --size 256
pkgpeek-cli card  juego.pkg card.png --width 420 --height 720
pkgpeek-cli shelltest target\debug\pkgpeek_shell.dll juego.pkg out\   # prueba los handlers COM sin registrarlos
pkgpeek-cli shellcheck juego.pkg out\                                 # consulta al shell ya instalado
```

**Diagnóstico**: si existe la carpeta `%USERPROFILE%\AppData\LocalLow\PkgPeek`, la extensión escribe ahí un `debug.log`. Se usa LocalLow porque es donde puede escribir el `prevhost.exe` de integridad baja que aloja la vista previa. Borra la carpeta para desactivar el log.

**Nota técnica**: los objetos COM hechos con `#[implement]` de windows-rs son *agile* por defecto (free-threaded marshaler). En un preview handler eso hace que `prevhost` llame a `DoPreview` desde un hilo worker de RPC: la ventana se crea en un hilo sin bucle de mensajes, nunca se pinta y cuelga el Explorador. Por eso todos los handlers usan `Agile = false`.
