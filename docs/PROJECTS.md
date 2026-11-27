# Projects: contrato JSON v1 y vínculo principal unificado

## Alcance y autoridad

`brain projects` implementa un flujo explícito para el host local. No tiene UI,
no llama modelos y no concede permisos por recibir un UUID o un booleano.
El host controla cwd efectivo, perfil, confirmación humana y controles de escritura.
Se usa exclusivamente el vault validado de `brain config`: no se acepta `vault`,
rutas de contexto, comandos o variables de entorno en esta superficie.

Un Project principal se selecciona en el **cwd exacto**, sin búsqueda de padres.
Fuentes adicionales elegidas por carpeta/perfil pertenecen a la UI de Pi:
Brain permite leerlas por identidad explícita; nunca les habilita escritura ni
las asocia a Memories. Esta API no implementa un registro de fuentes adicionales
ni la acción visual «Restablecer».

## Transporte

Una solicitud JSON hasta EOF en stdin; una respuesta JSON + LF en stdout.
Entrada/salida: 4 MiB. Objetos de argumentos rechazan campos desconocidos.
No hay preguntas interactivas. Fallos lógicos no requieren código de salida distinto
de cero; errores de transporte/proceso no son recibos de no publicación.

```json
{"protocol_version":1,"request_id":"550e8400-e29b-41d4-a716-446655440000","operation":"status","arguments":{}}
```

Respuesta:

```json
{"protocol_version":1,"request_id":"550e8400-e29b-41d4-a716-446655440000","operation":"status","result":{"status":"unbound","binding":"none","allow_write":false,"principal":null,"expected":{"vault_directory":"...","marker":null,"legacy_marker":null,"memory":{"status":"unbound","directory":"..."}}}}
```

`request_id` correlaciona la solicitud; no es una autorización ni una clave de
idempotencia global. Protocolo no compatible: `unsupported`. Argumentos/JSON
inválidos: `invalid`. Límites: `too_large`; no se trunca silenciosamente una
respuesta completa. Otros estados de fallo: `unavailable`, `not_found`, `denied`,
`busy`, `conflict`, `manual_review`, `partial`.

## Tipos y operaciones

- `Project = {project_id:UUID,title:string,path:string}`. `path` es relativo al vault.
- `Baseline = {vault_directory:string,marker:string|null,legacy_marker:string|null,memory:object}`.
  `vault_directory` es el vault canónico observado, no una selección de destino;
  debe coincidir con la configuración vigente al confirmar.
  Los marcadores son bytes UTF-8 exactos. `memory` es el resultado completo de
  `Store::resolve` con boundary igual al cwd. No es texto para enviar a un modelo:
  el host lo conserva privadamente y lo reenvía exacto al confirmar.
- `Preview = {project_id:UUID,title:string,body:string,path:string}`.
  Brain genera el UUID en preview y deriva el destino de título + UUID.
  El host devuelve el mismo preview, no un destino redactado por un modelo.

| Operación | Argumentos | Resultado exitoso |
|---|---|---|
| `capabilities` | `{}` | `status:found`, versión, operaciones, límites, formatos de historial |
| `status` | `{}` | `status:found` o `unbound`, `binding:unified|legacy|none`, `principal:Project|null`, `allow_write:boolean`, `expected:Baseline` |
| `list` | `{}` | `status:listed`, `projects:[Project]` |
| `read` | `{project_id}` | `status:found`, `project`, `markdown`, `body` |
| `update_body` | `{expected_project:Project,expected:EditScope,expected_markdown,body,human_approved:true}` | `status:updated\|unchanged`, `project` |
| `preview_create` | `{title,body}` | `status:preview`, `preview:Preview`, `expected:Baseline`; sin escritura |
| `draft_context` | `{}` | `status:found`, `memory`, `history`; sin selección arbitraria de Memory |
| `bind` | `{project_id,expected_project:Project,expected:Baseline,human_approved:true}` | `status:bound`, `project`, `allow_write:true`, `memory`, `created:false`, `notes_created:boolean` |
| `create_and_bind` | `{preview:Preview,expected:Baseline,human_approved:true}` | igual a bind, con `created:true` |

El host obtiene una vista inicial, permite editar título y cuerpo y solicita un
nuevo preview después de cada edición. Antes de ofrecer confirmación compara
la baseline de esa vista con `expected` del preview. Una sola confirmación humana
cubre la creación opcional, la asociación Memory y el vínculo principal.
El booleano representa esa decisión ya obtenida: Brain no autentica al humano.

Para vincular un Project existente, `expected_project:{project_id,title,path}` es
**obligatorio** y contiene exactamente los tres valores mostrados al usuario, de
`list/read`, sin reconstruirlos desde un listado posterior. Brain compara identidad,
título y ruta actuales con esa selección antes de inicializar locks si ya hay una
discrepancia, y vuelve a compararlos bajo el lock de publicación antes de crear
`notas/`, asociar Memory o publicar el marcador. Una discrepancia devuelve `conflict`;
no se adopta silenciosamente el nuevo título o destino para el mismo UUID. Omitir
`expected_project` devuelve `invalid`: no hay fallback al bind sin guard.
`create_and_bind` conserva su preview y no toma `expected_project`. Esto no añade
CAS contra un editor externo que cambie archivos después de la última comprobación.

`memory` del recibo es uno de:

- `{status:"associated",memory_id,revision}`: misma identidad, revisión nueva.
- `{status:"unchanged",memory_id,revision}`: asociación exacta ya existente.
- `{status:"deferred"}`: no hay Memory todavía; no se crea un documento vacío.

## Projects activos y compatibilidad

Se enumeran únicamente carpetas directas de `01-proyectos/`, y Markdown `.md`
directo dentro de cada carpeta. No se recorren `notas/`, áreas, archivos, recursos,
Memories ni destinos de wikilinks. No hay búsqueda de todo el vault.

Cada candidato `brain_type: project` debe tener metadatos v1 válidos, UUID no nulo,
título de hasta 256 bytes sin controles ni delimitadores de wikilink y ruta segura.
Solo puede haber una ficha Project por carpeta y un uso de cada UUID en el área.
Un Project v1 sin `project_status` se considera activo (compatibilidad).
`project_status: active` también es activo; `project_status: archived` se excluye.
Otros valores explícitos son inválidos: no se interpreta estado de Task ni se
inventa un migrador de formatos ajenos. Documentos no gestionados se omiten;
frontmatter malformado, identidad duplicada o ruta insegura falla cerrado.

Límites: 256 entradas por directorio inspeccionado, 512 KiB por ficha, 8 KiB por
marcador. Se rechazan symlinks; archivos administrados también rechazan hardlinks
en Unix. `.brain-projects.lock` es un sidecar reservado de publicación y se excluye
del conteo del área. El preview y la creación comprueban capacidad antes de añadir
una carpeta, para no producir la entrada 257. El lock del área serializa creaciones
cooperantes desde distintos cwd. No se descarga ni instala dependencia alguna.

## Creación y preview

Destino generado para fichas **nuevas**:
`01-proyectos/<slug>-<UUID>/<título>.md` con `notas/` vacía. Ejemplo de título
`StelaShell`: archivo `StelaShell.md`. El nombre de archivo conserva exactamente
mayúsculas, acentos, representación Unicode y espacios interiores del título; no
se normaliza, recorta, convierte a slug ni sustituye por un nombre genérico.

Solo la **carpeta** usa slug ASCII alfanumérico en minúsculas, separadores
normalizados y hasta 48 bytes; si no queda texto utilizable se usa `proyecto`.
Se conserva el UUID completo. La confirmación vuelve a validar el título y exige
la ruta exacta derivada de ese título y UUID, incluida la grafía del archivo.
Un preview con otro nombre, incluso `Proyecto.md`, devuelve `invalid` sin crear
carpeta, ficha, marcador ni sidecars de lock. Es necesario obtener un nuevo preview.

Validación de nombre **solo para preview/creación**, además de las reglas de título:

- Componente completo con el sufijo literal `.md`: máximo **255 bytes UTF-8 y
  255 unidades UTF-16**. Por ejemplo, 252 letras ASCII + `.md` caben; 253 no.
  No se limita solo el título ni se trunca para ajustarlo.
- No vacío, controles, separadores `/` o `\`, caracteres `: * ? " < > |`,
  delimitadores `[`/`]`/`#`, espacios Unicode en los extremos ni punto al final
  del título. Los espacios interiores portables se mantienen exactamente.
- Reservados sin distinguir mayúsculas: `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`,
  `LPT1`–`LPT9`, `CONIN$`, `CONOUT$` y, por política conservadora, `CLOCK$`.
  También se rechazan variantes con extensiones (`NUL.txt`), espacios antes del
  primer punto (`CON .txt`) y dígitos elevados `¹`, `²`, `³` tras `COM`/`LPT`.
  `COM0`, `COM10` y `LPT10` no se consideran dispositivos reservados.

Un nombre no portable devuelve `status:invalid`, `reason:INVALID_PROJECT_FILENAME`
y feedback para **editar el título**, sin escrituras ni sustitución silenciosa.
Esta política no cambia `catalog`, la validación de marcadores ni fichas existentes:
`Proyecto.md` sigue siendo válido, y los títulos visibles existentes no se someten
retroactivamente a las restricciones de un nombre nuevo. No se renombran fichas.
El límite es de componente, no una garantía sobre la longitud total del path en
todas las configuraciones/plataformas.

Pi propone un título editable desde el nombre de carpeta y redacta contenido en
español. Brain valida estructura, identidad, límites y seguridad;
**no clasifica automáticamente el idioma**.
Cuerpo sin frontmatter, con H2 top-level reales, exactamente en orden:

```markdown
## Summary
Resumen del proyecto.
## Objective
Objetivo concreto.
## Scope
Incluido y excluido.
## References
Antecedentes opcionales.
```

`References` es opcional. Fences/citas no sustituyen a esos encabezados. Brain
administra el frontmatter `brain_type: project`, versión 1, UUID y título.
Seleccionar una ficha existente no modifica ni reserializa sus bytes.
Si al vincular una ficha existente falta `notas/`, se crea únicamente esa carpeta
vacía, dentro de la misma confirmación; una entrada no regular o symlink se rechaza.

### Editar el cuerpo de una ficha existente

`update_body` es una operación separada y solo sirve para el principal activo con
`allow_write: true` del cwd exacto. El host toma `expected_project` y los bytes
íntegros de `read` (`expected_markdown`), y toma `EditScope` de la respuesta de
`status`: `vault_directory`, `marker` y `legacy_marker`, sin `memory`. Después de
revisar y confirmar el nuevo cuerpo no se vuelve a capturar la base.

Brain compara el ámbito, el Project y los bytes originales antes de preparar la
escritura y bajo los locks cooperativos de vínculo y Projects. Solo reemplaza el
cuerpo, preservando título, UUID, ruta y bytes del frontmatter, salvo añadir el
salto de línea del delimitador si estaba al final de un archivo sin cuerpo. No
cambia Memory, historial, notas ni marcadores y no crea `notas/`. Valen los mismos
encabezados y el límite total de 512 KiB. No se garantiza CAS contra editores
externos ni la conservación de metadatos de filesystem ajenos al documento.
Brain verifica la lectura posterior antes de devolver `updated`. Ante `partial`
o recibo incierto, inspeccionar el estado; no reintentar ni restaurar automáticamente.

## Marcador unificado y legado

```toml
binding_version = 2
# Omitido hasta el primer guardado útil si todavía no hay Memory:
memory_id = "550e8400-e29b-41d4-a716-446655440001"
memory_subfolders = ["child"]

[principal_project]
project_id = "550e8400-e29b-41d4-a716-446655440002"
project_path = "01-proyectos/ejemplo/Proyecto.md"
allow_write = true
```

- `.brain.toml` v1 de Memory y `.brain-notes.toml` v1 siguen legibles.
  Leerlos no migra ni crea un marcador. Solo bind explícito publica v2.
- V2 requiere un `principal_project` completo. Un v2 incompleto/ilegible falla
  cerrado, nunca habilita fallback al legacy ni escritura por defecto.
- V2 válido gana sobre el legacy, que queda inactivo y no se modifica ni elimina.
  Su lectura no se exige para Notes ni se incluye en baseline (`legacy_marker:null`).
  En v1/sin unificado el legacy es parte exacta de baseline y conserva su
  `allow_write` explícito, por defecto false. El host distingue ambos con `binding`.
- El bind confirmado fija `allow_write:true` en el principal, sujeto a permisos
  externos. Reasociar conserva UUID de Memory y declaraciones de hijos; no migra
  otras copias del marcador ni crea una nueva identidad.
- Cambios de marcadores/Memory entre vista y confirmación devuelven `conflict`.
  Memory manual, parcial o corrupta bloquea el bind antes de publicación funcional.
- Brain conserva bytes ajenos al editar los campos propios; TOML/YAML ambiguo o
  comentarios mezclados en tablas/scalars propios que no pueden aislarse sin
  pérdida se rechazan para revisión, no se normalizan.
- `brain memory resolve` **mantiene proyección wire v1** en `binding`, aunque disco
  sea v2: `binding_version:1`, `memory_id`, `memory_subfolders`. V2 sin Memory se
  resuelve como unbound. El primer create útil incorpora el principal validado y
  agrega `memory_id` al marcador, preservando el resto y usando reserva/intent
  existentes. No se crea Memory durante bind. Los hijos explícitos siguen lazy.

## Asociación y parcialidad

La Memory existente se bloquea, valida contra revisión **y bytes aceptados**, y
publica con el pipeline de snapshots/log, baseline e intent existente. No se usan
`create`, `replace_missing` ni `accept_external` para cambiar su Project.

La operación modifica solo revisión, scalars propios `brain_project`/`project_link`
y una relación corporal delimitada bajo Current decisions. Propiedades desconocidas,
comentarios ajenos y referencias humanas previas se conservan. Un bloque reservado
ambiguo/literal/editado se rechaza. Historial v1 conserva snapshots previos;
historial v2 conserva entradas previas del log y publica la nueva baseline aceptada.
Esto no añade retención de snapshots completos a v2.

No existe transacción multiarquivo ni CAS contra editores arbitrarios. Orden:
reserva exclusiva de carpeta si se crea, ficha/notas, asociación Memory, marcador.
Una interrupción tras creación puede dejar Project sin vínculo; una interrupción
tras asociación puede dejar Memory asociada con el marcador principal anterior.

Recibo parcial: `{status:"partial",project_created:boolean,project_reserved:boolean,
notes_created:boolean,project:Project,reason,message}`. `project_reserved:true` con
`project_created:false` indica carpeta reservada pero ficha no publicada. No debe
mostrarse como un Project creado satisfactoriamente.

No se elimina ni sobrescribe contenido para compensar fallos. Mismo preview reutiliza
UUID/ruta: si ya existen devuelve conflicto o parcial, **nunca una segunda carpeta**.
Si ficha quedó creada, el host conserva el recibo, refresca explícitamente y permite
solo selección/bind explícito de esa identidad; no repite create_and_bind en automático.
Si solo quedó reserva sin ficha, se requiere inspección humana: no reparación inventada.
Sin recibo válido/correlacionado se mantiene fence de publicación no confirmada;
una lectura posterior no confirma retroactivamente el recibo perdido.

## Contexto para borrador

`draft_context` solo resuelve la Memory del cwd exacto seleccionado por Brain.
No toma UUID, cursor ni path de otra Memory. Devuelve el estado completo de resolve
y una porción final de su propio `Historial.md` validado:

```text
history = {status:"found",format:1|2,markdown:string,
           returned_bytes:number,total_bytes:number,truncated:boolean}
       o {status:"unavailable",reason:string}
```

El fragmento final tiene hasta 32768 bytes UTF-8, sin cortar un carácter, pero puede
comenzar dentro de una entrada: no se presenta como una entrada completa ni como
un historial completo si `truncated:true`. Soporta snapshots v1 y log v2. No escribe,
no adopta ediciones manuales y no consulta otras Memories. Si el historial no está
disponible se permite devolver ese estado; la UI decide si ofrece generar borrador.

El límite es **de respuesta del historial**: la validación previa de baseline sigue
leyendo los archivos completos como el store existente. No hay promesa de IO acotada,
latencia ni retención. El transporte rechaza una Memory íntegra que exceda 4 MiB.
Pi debe mostrar un aviso antes de enviar al proveedor solo Memory vigente y el
fragmento pertinente autorizado; no enviar baseline privada, paths ni otros documentos.

## Notes adicionales

`brain notes` conserva su envelope v1 anterior (sin campo operation en respuesta).
Se amplían solo lecturas: `list {project_id?:UUID}` y `read {note_id,project_id?:UUID}`.
Sin ID usan principal exacto; con ID validan Project activo solicitado explícitamente
y fuerzan scope de lectura. `create/update` siguen cargando exclusivamente el principal
y exigiendo su ID/permisos; un UUID adicional nunca habilita escrituras.

## Evidencia de esta unidad

Regresiones preparadas en `tests/cli_projects.rs`, `tests/memory_store.rs` y tests
unitarios de marcador/asociación. Se adapta un fixture de Notes para representar
un marcador Memory v1 válido; se añaden negativos v2 explícitos. Pruebas, builds,
suites y labs **no ejecutados** por el alcance actual. Revisión estática y
`git diff --check` no equivalen a compilación ni verificación runtime.
