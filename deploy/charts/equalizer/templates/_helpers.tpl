{{/*
Expand the resource base name.

Resources use a fixed name (not release-prefixed) so the chart renders
field-equivalent to the Kustomize bundle and the pre-migration raw manifests
(spec-015, research R3).
*/}}
{{- define "equalizer.name" -}}
capacity-equalizer
{{- end -}}

{{/*
Common labels shared across equalizer resources. Mirrors the single `app` label
used by the raw manifests so selectors stay identical.
*/}}
{{- define "equalizer.labels" -}}
app: capacity-equalizer
{{- end -}}

{{/*
Namespace for namespaced resources.
*/}}
{{- define "equalizer.namespace" -}}
{{- .Values.namespace -}}
{{- end -}}
