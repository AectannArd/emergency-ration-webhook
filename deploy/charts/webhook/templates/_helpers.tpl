{{/*
Expand the resource base name.

Resources use a fixed name (not release-prefixed) so the chart renders
field-equivalent to the Kustomize bundle and the pre-migration raw manifests
(spec-015, research R3).
*/}}
{{- define "webhook.name" -}}
capacity-admission-webhook
{{- end -}}

{{/*
Common labels shared across webhook resources. Mirrors the single `app` label
used by the raw manifests so selectors stay identical.
*/}}
{{- define "webhook.labels" -}}
app: capacity-admission-webhook
{{- end -}}

{{/*
Namespace for namespaced resources.
*/}}
{{- define "webhook.namespace" -}}
{{- .Values.namespace -}}
{{- end -}}
