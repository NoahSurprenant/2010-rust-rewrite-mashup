{{/* The objects' name: fullnameOverride, else the release name if it contains the chart's name, else
   <release>-<chart>. */}}
{{- define "iw4l-master.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end }}

{{- define "iw4l-master.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/* Labels on every object. */}}
{{- define "iw4l-master.labels" -}}
{{ include "iw4l-master.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/* What the Service and the NetworkPolicy select: stable across chart versions. */}}
{{- define "iw4l-master.selectorLabels" -}}
app.kubernetes.io/name: {{ include "iw4l-master.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/* repository:tag, repository:tag@digest, or repository@digest. tag may itself carry a digest
   (tag@sha256:... or sha256:..., what Argo CD Image Updater writes); digest, when set, replaces it.
   An empty tag is the chart's appVersion. */}}
{{- define "iw4l-master.image" -}}
{{- $image := .Values.image -}}
{{- $ref := $image.repository -}}
{{- $tag := toString ($image.tag | default .Chart.AppVersion) -}}
{{- if $image.digest }}{{ $tag = regexReplaceAll "@.*$" $tag "" }}{{ if hasPrefix "sha256:" $tag }}{{ $tag = "" }}{{ end }}{{ end -}}
{{- if hasPrefix "sha256:" $tag }}{{ $ref = printf "%s@%s" $ref $tag }}
{{- else if $tag }}{{ $ref = printf "%s:%s" $ref $tag }}{{ end -}}
{{- if and $image.digest (not (contains "@" $ref)) }}{{ $ref = printf "%s@%s" $ref $image.digest }}{{ end -}}
{{- $ref -}}
{{- end }}

{{/* The Secret the master's certificate and key are in: tls.existingSecret, else the one
   cert-manager writes. Fails when there is neither. */}}
{{- define "iw4l-master.tlsSecret" -}}
{{- if .Values.tls.existingSecret -}}
{{- .Values.tls.existingSecret -}}
{{- else if .Values.tls.certManager.enabled -}}
{{- .Values.tls.certManager.secretName | default (printf "%s-tls" (include "iw4l-master.fullname" .)) -}}
{{- else -}}
{{- fail "tls: set existingSecret, or certManager.enabled - the master needs a certificate and key" -}}
{{- end -}}
{{- end }}

{{/* "true" when the chart makes the certificate (cert-manager, no existing Secret). */}}
{{- define "iw4l-master.certManager" -}}
{{- if and .Values.tls.certManager.enabled (not .Values.tls.existingSecret) }}true{{ end -}}
{{- end }}

{{/* The self-signed Issuer's name, when the chart makes one. */}}
{{- define "iw4l-master.issuerName" -}}
{{- printf "%s-selfsigned" (include "iw4l-master.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/* The file the probes trust: the certificate itself when it is self-signed (an existing Secret,
   or the chart's own issuer), else the issuer's CA, which cert-manager writes as ca.crt. */}}
{{- define "iw4l-master.probeCa" -}}
{{- if and (include "iw4l-master.certManager" .) .Values.tls.certManager.issuerRef -}}
/etc/iw4l/tls/ca.crt
{{- else -}}
/etc/iw4l/tls/tls.crt
{{- end -}}
{{- end }}
