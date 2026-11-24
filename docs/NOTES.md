# Notas de proyecto: contrato JSON v1

`brain notes` intercambia un objeto JSON por invocación: solicitud hasta EOF en
stdin, respuesta en una línea en stdout. No muestra preguntas. Es una superficie
separada de `brain memory`; Notes no modifica una vinculación de Memory.
El flujo explícito `brain projects` sí puede seleccionar principal y asociar Memory:
ver [PROJECTS.md](PROJECTS.md).

## Configuración del operador

El host elige el directorio de trabajo efectivo y el perfil de Brain. Un operador
con autoridad configura el vault con `brain config`. El principal confirmado se
lee del `.brain.toml` unificado v2 **en el directorio de trabajo exacto**. Sin v2,
se conserva la lectura del `.brain-notes.toml` v1 legado mostrado abajo. Leer no migra
marcadores; v2 inválido/incompleto nunca hace fallback al legado. V2 válido deja el
legacy inactivo, sin exigir su lectura. No hay búsqueda en ancestros ni creación de
contexto por leer Notes. Un marcador Memory v1 válido por sí solo no concede acceso
a un Project. La solicitud no admite
rutas, flags, entorno ni elección de vault. Un adaptador no debe ofrecer al modelo
la edición del marcador, la selección del perfil o una vía de ejecución libre
como sustituto del permiso de escritura.

```toml
binding_version = 1
project_id = "b341b410-5200-46f5-9ea3-e34caa551005"
project_path = "01-proyectos/mi-proyecto/Mi proyecto.md"
allow_write = false
```

`project_path` es relativo al vault configurado y tiene exactamente esos tres
componentes. El archivo debe ser Markdown UTF-8, `brain_type: project`, con
`brain_version` compatible y `brain_id` igual a `project_id`. No puede haber
otro documento `project` directo en la misma carpeta. La carpeta `notas/`, hija
directa del Project, debe existir. Para habilitar escritura, el operador debe
optar por `allow_write = true` en el legado (omitirlo equivale a `false`). El bind
unificado confirmado lo habilita por defecto solo en el principal. Una integración
puede y debe aplicar su propio permiso de escritura antes de invocar Brain.

La pertenencia se define por **ubicación física en ese `notas/` dedicado** y
por metadatos de Note (`brain_type: note`, `brain_version`, UUID no nulo y
`title`), no por enlaces ni por un metadato de pertenencia. Solo se consideran
archivos Markdown `.md` directos; no hay recursión, resolución de wikilinks,
búsqueda exterior, Tasks, Memory, áreas o recursos. Identidades Note duplicadas
en el mismo `notas/` producen conflicto. Un Markdown sin frontmatter, sin tipo
gestionado relevante o de otro tipo no se incluye. Frontmatter malformado y
candidatos no regulares producen un fallo en vez de omitir una posible ambigüedad.
El marcador y los candidatos no pueden ser symlinks (tampoco hardlinks en Unix);
los directorios en la ruta administrada bajo el vault tampoco pueden ser symlinks.

## Mensajes

```json
{"protocol_version":1,"request_id":"b341b410-5200-46f5-9ea3-e34caa551009","operation":"list","arguments":{}}
```

`request_id` es un UUID no nulo elegido por el host para correlacionar la
respuesta; **no** es una clave de idempotencia. La respuesta contiene
`{"protocol_version":1,"request_id":…, "result":{…}}`. Si no se pudo
decodificar la envoltura completa o se superó el límite de entrada, el `request_id`
de respuesta es `null`. Campos desconocidos en la envoltura o argumentos se
rechazan.

| Operación | `arguments` | `result` exitoso |
|---|---|---|
| `list` | `{project_id?:UUID}` | `status: "listed"`, `project_id`, `notes: [{note_id, title}]` |
| `read` | `{note_id, project_id?:UUID}` | `status: "found"`, `project_id`, `note_id`, `title`, `markdown`, `body` |
| `create` | `{project_id, title, body, require_named_link?:true}` | `status: "created"`, `project_id`, `note_id`, `title` |
| `update` | `{project_id, note_id, expected_markdown, body}` | `status: "updated"` o `"unchanged"`, `project_id`, `note_id`, `title` |
| `rename` | `{project_id, note_id, expected_markdown, title}` | `status: "renamed"` o `"unchanged"`, `project_id`, `note_id`, `title` |
| `inspect_rename` | `{project_id}` (principal) | `status: "no_intent"` o `"partial"`; véase recuperación |

- En `list/read`, un `project_id` explícito permite una fuente adicional activa
  validada por identidad dentro de `01-proyectos`; ese scope es siempre de lectura.
  Sin ID se usa el principal. `create/update/rename` nunca usan scopes adicionales.
- `list` no incluye rutas ni cuerpos. `read` retorna el documento UTF-8 exacto
  en `markdown` y el cuerpo sin frontmatter en `body`. El host conserva el
  `markdown` exacto de la lectura que sirve de base, sin normalizar saltos de
  línea, y no acepta una base inventada por el modelo.
- En escrituras, el host envía el `project_id` observado de su contexto.
  Debe invalidar contexto y bases cuando cambia de perfil, vault o vinculación.
  Brain comprueba la coincidencia del Project, no solo la existencia de la nota.
- `create` conserva el UUID como identidad interna (`brain_id`), pero publica
  el archivo `<título>.md` en `notas/` con el título exacto (sin sustituir
  caracteres ni normalizar Unicode). Brain genera la propiedad `project_link`
  como wikilink entrecomillado a la ruta de la ficha del Project principal
  dentro del vault, por ejemplo `project_link: "[[01-proyectos/A/Proyecto]]"`.
  La propiedad sirve de enlace en Obsidian; no concede acceso ni altera el
  cuerpo que devuelve `read`. La autorización sigue naciendo del vínculo.
- El host puede exigir esa forma de publicación con `require_named_link: true`.
  Un Brain antiguo rechaza el campo desconocido antes de escribir: no hay
  fallback. Los clientes que omiten el campo todavía crean notas con el
  nombre y enlace nuevos usando un Brain compatible.
- Un título ya usado por una nota del Project (comparado en minúsculas) o un
  destino ocupado devuelve `conflict`: nunca se cambia de nombre ni se
  sustituye un archivo. Las notas existentes, incluidas `<UUID>.md` sin
  enlace, se siguen leyendo y actualizando por ID sin migración automática.
  `body` no incluye el frontmatter que administra Brain.
- `update` exige coincidencia exacta de `expected_markdown` y solo sustituye
  `body`. Deja intacto el frontmatter (incluidos comentarios y campos ajenos).
  Si el delimitador final estaba en EOF sin salto de línea y se añade contenido,
  inserta el salto separador necesario. No renombra ni cambia el título.
- Solo lecturas explícitas devuelven el documento. El recibo de una escritura
  no es una lectura posterior ni una garantía de que otro editor no lo haya
  cambiado desde entonces.

### Renombrado y recuperación

`rename` exige el Project principal con escritura habilitada y una base
`expected_markdown` exacta. El mismo título exacto devuelve `unchanged` y no
migra el nombre legado. Un título diferente cambia solo el valor YAML de
`title` y publica en `<título>.md`. Conserva UUID, `project_link`, frontmatter
ajeno y cuerpo exactos. No busca ni cambia enlaces entrantes. Para evitar una
reserialización, admite un `title` de línea única (escalar simple o cadena JSON
entre comillas); otras formas, incluyendo comentarios al final de esa línea,
se rechazan si no puede aislarse el valor. Usa las reglas portables de `create` y
rechaza colisiones de título o destino. Un cambio solo de mayúsculas devuelve
`conflict` si el sistema considera ambas rutas la misma entrada existente.
Si el título no coincide pero la ruta ya es exactamente la deseada, se
reemplaza únicamente el archivo de origen que coincide con la base.

Para publicar un cambio de ruta, Brain retiene `.brain-note-rename.json`
con las rutas directas y los documentos exactos de antes/después. Publica el
destino sin sobrescribir, verifica el resultado y solo entonces retira el
origen que aún coincide con la base. Retira el intento al final, tras verificar
el destino. Esto no es una transacción de dos archivos para observadores
externos, versiones antiguas ni ante cortes de energía. No usar esas versiones
ni editores externos durante el renombrado o la reconciliación.

`list/read` obtienen un lock compartido cuando existe y pueden devolver `busy`
si hay un escritor. Un intento retenido bloquea las operaciones ordinarias de
Notes en ese Project, incluidas lecturas adicionales, con `partial`.
`inspect_rename` no escribe; para un intento válido devuelve `partial` con
`project_id`, `note_id`, nombres `source` y `destination`, y `source_state` y
`destination_state`: `before`, `after`, `absent`, `different` o `unavailable`.
Un intento ilegible o inválido también produce `partial`, sin clasificar rutas.
Sin intento devuelve `no_intent`; eso no confirma un recibo perdido. No hay
reparación automática ni comando de recuperación: el operador debe revisar las
copias exactas retenidas y los archivos, fuera de concurrencia, antes de decidir
cómo reconciliarlos. No borrar el intento para desbloquear sin esa revisión.

### Límites y fallos

Entrada y salida: 4 MiB (incluido el salto de línea de respuesta). Cada archivo
Markdown candidato: 512 KiB; cada directorio inspeccionado: 256 entradas directas
(excluyendo el archivo reservado `.brain-notes.lock`). El marcador admite 8 KiB.
Los títulos existentes de Note no pueden estar en blanco, contener caracteres de
control ni exceder 256 bytes UTF-8. En creación, `<título>.md` debe caber en
255 bytes UTF-8 y 255 unidades UTF-16, y se rechazan los títulos con separadores,
caracteres no portables, espacios en los extremos, punto inicial o final y
nombres de dispositivo reservados de Windows. No hay truncamiento ni paginación.

Los errores tienen `result: {status, message}` con uno de `invalid`,
`unsupported`, `unbound`, `unavailable`, `denied`, `not_found`, `conflict`,
`busy`, `input_limit` u `output_limit`. Una respuesta completa y correlacionada
con uno de estos errores no publicó una nota. `partial` es distinto: una
publicación de renombrado puede haberse iniciado y requiere inspección. `output_limit` puede ocurrir en
una lectura (por ejemplo, los escapes JSON expanden el contenido), pero no en
un recibo de escritura. Una escritura con error aún puede haber inicializado
el archivo de bloqueo. Si falta el recibo tras enviar una escritura, no coincide
su ID o llega truncado, el resultado es **no confirmado**: no repetir en
automático. Una lectura explícita puede reconciliar el contenido observable,
pero no confirma retroactivamente el recibo perdido.

Brain serializa escritores que cooperan mediante `.brain-notes.lock` y publica
el archivo preparado atómicamente en el mismo directorio. Vuelve a validar la
vinculación y, para una actualización, comprueba los bytes de la nota antes de
publicar. El operador controla el árbol; esto no es un sandbox contra un proceso
que sustituye entradas de ruta entre comprobaciones. Los editores externos
no respetan necesariamente el lock: no deben escribir en paralelo, pues no hay
un compare-and-swap del sistema de archivos contra esos editores entre la última
comprobación y la publicación.

## Verificación aislada

`cargo test --locked --test cli_notes` usa un directorio temporal con vault,
perfil y contexto propios; no toca el vault ni el binario instalados.
