# Brain: preparar y seleccionar un vault

`brain init <ruta>` prepara las ocho carpetas PARA/sistema; **no selecciona** el vault activo.

```sh
brain init ./mi-vault
brain config set-vault ./mi-vault
brain config show
```

La selección exige un directorio existente con la estructura completa de `init`.
Una ruta relativa se convierte en absoluta. Cambiar una selección requiere
confirmación interactiva o `brain config set-vault <ruta> --yes` sin terminal;
`--yes` no omite la validación. Seleccionar el mismo vault válido no reescribe el archivo.

## Archivo de configuración

- Linux/Unix: `$HOME/.config/brain/config.toml`.
- Windows: `%USERPROFILE%\.config\brain\config.toml`.

La variable de la plataforma debe contener una ruta absoluta no vacía.
`XDG_CONFIG_HOME` se ignora: no hay respaldo en otra variable ni en el directorio
actual, y no se migran automáticamente configuraciones anteriores.

```toml
config_version = 1
vault_path = '/ruta/absoluta/al/vault'
```

`config_version` indica la versión compatible; `vault_path` es el vault completo.
`config show` lee y valida ambos campos sin crear ni reparar archivos.

## Notas de proyecto

`brain notes` es la interfaz JSON local para listar, leer, crear y actualizar
notas del proyecto principal seleccionado por el operador, con lectura explícita
de proyectos adicionales sin habilitar escritura en ellos. Consulta el
[contrato de Notas](docs/NOTES.md) para configurar un contexto y separar el
permiso de escritura de la lectura. Notes no modifica la interfaz de Memory.

## Selección explícita de Project

`brain projects` expone JSON v1 para listar Projects activos, consultar una ficha,
previsualizar creación y confirmar el vínculo principal del cwd exacto. Usa
`.brain.toml` unificado v2, con lectura compatible de marcadores v1 y Notes legado,
sin migración implícita. Asocia la Memory existente sin recrearla, o difiere su
creación hasta el primer guardado útil. También permite leer una porción acotada
del historial de la Memory seleccionada para un borrador autorizado por el host.

El [contrato de Projects](docs/PROJECTS.md) define argumentos, recibos, confirmación,
preflight de baseline, límites, acceso adicional de solo lectura y parcialidad.
No implementa la UI de Pi ni concede permisos por recibir un UUID o un booleano.

## Prueba manual aislada (shell Unix)

Compila con `cargo build` **antes** de aislar las variables; no exportes `HOME` para Cargo.

```sh
bin="$PWD/target/debug/brain"
lab="$(mktemp -d)"
run() { HOME="$lab/user" USERPROFILE="$lab/user" "$bin" "$@"; }
run init "$lab/vault"
run config set-vault "$lab/vault"
run config show
```

El archivo de esta prueba queda en `$lab/user/.config/brain/config.toml`,
no en la configuración real del usuario. Aislar solo XDG ya no protege esa configuración.
