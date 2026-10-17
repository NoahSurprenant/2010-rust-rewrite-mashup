{{/* The objects' name: fullnameOverride, else the release name if it contains the chart's name, else
   <release>-<chart>. */}}
{{- define "iw4l-dedicated.fullname" -}}
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

{{- define "iw4l-dedicated.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/* Labels on every object. */}}
{{- define "iw4l-dedicated.labels" -}}
{{ include "iw4l-dedicated.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/* What the Service and the NetworkPolicy select: stable across chart versions. */}}
{{- define "iw4l-dedicated.selectorLabels" -}}
app.kubernetes.io/name: {{ include "iw4l-dedicated.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/* repository:tag, repository:tag@digest, or repository@digest. tag may itself carry a digest
   (tag@sha256:... or sha256:..., what Argo CD Image Updater writes); digest, when set, replaces it.
   An empty tag is the chart's appVersion. */}}
{{- define "iw4l-dedicated.image" -}}
{{- $image := .Values.image -}}
{{- $ref := $image.repository -}}
{{- $tag := toString ($image.tag | default .Chart.AppVersion) -}}
{{- if $image.digest }}{{ $tag = regexReplaceAll "@.*$" $tag "" }}{{ if hasPrefix "sha256:" $tag }}{{ $tag = "" }}{{ end }}{{ end -}}
{{- if hasPrefix "sha256:" $tag }}{{ $ref = printf "%s@%s" $ref $tag }}
{{- else if $tag }}{{ $ref = printf "%s:%s" $ref $tag }}{{ end -}}
{{- if and $image.digest (not (contains "@" $ref)) }}{{ $ref = printf "%s@%s" $ref $image.digest }}{{ end -}}
{{- $ref -}}
{{- end }}

{{/* The claim of the data volume: persistence.existingClaim, else the chart's. */}}
{{- define "iw4l-dedicated.dataClaim" -}}
{{- .Values.persistence.existingClaim | default (printf "%s-data" (include "iw4l-dedicated.fullname" .)) -}}
{{- end }}

{{/* "true" when a game data volume is configured. */}}
{{- define "iw4l-dedicated.hasGameData" -}}
{{- with .Values.gameData -}}
{{- if or .existingClaim (and .nfs.server .nfs.path) .volume }}true{{ end -}}
{{- end -}}
{{- end }}

{{/* "true" when a cache volume is configured. */}}
{{- define "iw4l-dedicated.hasCache" -}}
{{- with .Values.cache -}}
{{- if or .existingClaim .volume }}true{{ end -}}
{{- end -}}
{{- end }}

{{/* The master's UDP port, from master.address (<host>:<port> or [<v6>]:<port>). */}}
{{- define "iw4l-dedicated.masterPort" -}}
{{- $port := regexFind ":[0-9]{1,5}$" .Values.master.address | trimPrefix ":" -}}
{{- if not $port -}}
{{- fail "master.address: give <host>:<port>, e.g. iw4l-master.iw4l-master.svc.cluster.local:4433" -}}
{{- end -}}
{{- $port -}}
{{- end }}
