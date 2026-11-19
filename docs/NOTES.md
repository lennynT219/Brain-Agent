# Notas de proyecto: contrato JSON v1

`brain notes` intercambia un objeto JSON por invocación: solicitud hasta EOF en
stdin, respuesta en una línea en stdout. No muestra preguntas. Es una superficie
separada de `brain memory`; no busca ni cambia una vinculación de Memory.

## Configuración del operador

El host elige el directorio de trabajo efectivo y el perfil de Brain. Un operador
con autoridad configura el vault con `brain config` y crea `.brain-notes.toml`
**en el directorio de trabajo exacto**. No hay búsqueda en ancestros, fallback,
creación de contexto ni lectura de marcadores de Memory. La solicitud no admite
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
optar por `allow_write = true` (omitirlo equivale a `false`). Una integración
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
| `list` | `{}` | `status: "listed"`, `project_id`, `notes: [{note_id, title}]` |
| `read` | `{note_id}` | `status: "found"`, `project_id`, `note_id`, `title`, `markdown`, `body` |
| `create` | `{project_id, title, body}` | `status: "created"`, `project_id`, `note_id`, `title` |
| `update` | `{project_id, note_id, expected_markdown, body}` | `status: "updated"` o `"unchanged"`, `project_id`, `note_id`, `title` |

- `list` no incluye rutas ni cuerpos. `read` retorna el documento UTF-8 exacto
  en `markdown` y el cuerpo sin frontmatter en `body`. El host conserva el
  `markdown` exacto de la lectura que sirve de base, sin normalizar saltos de
  línea, y no acepta una base inventada por el modelo.
- En escrituras, el host envía el `project_id` observado de su contexto.
  Debe invalidar contexto y bases cuando cambia de perfil, vault o vinculación.
  Brain comprueba la coincidencia del Project, no solo la existencia de la nota.
- `create` genera un UUID y un nombre de archivo nuevos; nunca sustituye un
  archivo existente. `body` no incluye el frontmatter que administra Brain.
- `update` exige coincidencia exacta de `expected_markdown` y solo sustituye
  `body`. Deja intacto el frontmatter (incluidos comentarios y campos ajenos).
  Si el delimitador final estaba en EOF sin salto de línea y se añade contenido,
  inserta el salto separador necesario. No renombra ni cambia el título.
- Solo lecturas explícitas devuelven el documento. El recibo de una escritura
  no es una lectura posterior ni una garantía de que otro editor no lo haya
  cambiado desde entonces.

### Límites y fallos

Entrada y salida: 4 MiB (incluido el salto de línea de respuesta). Cada archivo
Markdown candidato: 512 KiB; cada directorio inspeccionado: 256 entradas directas
(excluyendo el archivo reservado `.brain-notes.lock`). El marcador admite 8 KiB.
Los títulos de Note no pueden estar en blanco, contener caracteres de control ni
exceder 256 bytes UTF-8. No hay truncamiento ni paginación.

Los errores tienen `result: {status, message}` con uno de `invalid`,
`unsupported`, `unbound`, `unavailable`, `denied`, `not_found`, `conflict`,
`busy`, `input_limit` u `output_limit`. Una respuesta completa y correlacionada
con uno de estos errores no publicó una nota. `output_limit` puede ocurrir en
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
