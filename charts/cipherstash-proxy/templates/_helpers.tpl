{{/*
Expand the name of the chart.
*/}}
{{- define "cipherstash-proxy.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "cipherstash-proxy.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "cipherstash-proxy.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "cipherstash-proxy.labels" -}}
helm.sh/chart: {{ include "cipherstash-proxy.chart" . }}
{{ include "cipherstash-proxy.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "cipherstash-proxy.selectorLabels" -}}
app.kubernetes.io/name: {{ include "cipherstash-proxy.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "cipherstash-proxy.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "cipherstash-proxy.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Create namespace name
*/}}
{{- define "cipherstash-proxy.namespace" -}}
{{- if .Values.namespace.create }}
{{- .Values.namespace.name }}
{{- else }}
{{- .Release.Namespace }}
{{- end }}
{{- end }}

{{/*
Create image name with tag
*/}}
{{- define "cipherstash-proxy.image" -}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) }}
{{- end }}

{{/*
Get database password secret name and key
*/}}
{{- define "cipherstash-proxy.databasePasswordSecret" -}}
{{- if .Values.secrets.create -}}
{{- printf "%s-secrets" (include "cipherstash-proxy.fullname" .) -}}
{{- else -}}
{{- .Values.secrets.external.databasePasswordSecret.name -}}
{{- end -}}
{{- end }}

{{- define "cipherstash-proxy.databasePasswordSecretKey" -}}
{{- if .Values.secrets.create -}}
database-password
{{- else -}}
{{- .Values.secrets.external.databasePasswordSecret.key -}}
{{- end -}}
{{- end }}

{{/*
Get CipherStash client key secret name and key
*/}}
{{- define "cipherstash-proxy.cipherstashClientKeySecret" -}}
{{- if .Values.secrets.create -}}
{{- printf "%s-secrets" (include "cipherstash-proxy.fullname" .) -}}
{{- else -}}
{{- .Values.secrets.external.cipherstashClientKeySecret.name -}}
{{- end -}}
{{- end }}

{{- define "cipherstash-proxy.cipherstashClientKeySecretKey" -}}
{{- if .Values.secrets.create -}}
cipherstash-client-key
{{- else -}}
{{- .Values.secrets.external.cipherstashClientKeySecret.key -}}
{{- end -}}
{{- end }}

{{/*
Get CipherStash client access key secret name and key
*/}}
{{- define "cipherstash-proxy.cipherstashClientAccessKeySecret" -}}
{{- if .Values.secrets.create -}}
{{- printf "%s-secrets" (include "cipherstash-proxy.fullname" .) -}}
{{- else -}}
{{- .Values.secrets.external.cipherstashClientAccessKeySecret.name -}}
{{- end -}}
{{- end }}

{{- define "cipherstash-proxy.cipherstashClientAccessKeySecretKey" -}}
{{- if .Values.secrets.create -}}
cipherstash-client-access-key
{{- else -}}
{{- .Values.secrets.external.cipherstashClientAccessKeySecret.key -}}
{{- end -}}
{{- end }} 