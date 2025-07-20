# CipherStash Proxy Helm Chart

A Helm chart for deploying CipherStash Proxy - a transparent encryption proxy for PostgreSQL databases.

## Overview

CipherStash Proxy enables transparent encryption and searchable encryption capabilities for PostgreSQL databases without requiring application code changes. This Helm chart deploys the proxy in a Kubernetes cluster.

## Prerequisites

- Kubernetes 1.16+
- Helm 3.2.0+
- A PostgreSQL database accessible from the cluster
- CipherStash workspace credentials

## Installation

[Helm](https://helm.sh) must be installed to use the charts. Please refer to Helm's [documentation](https://helm.sh/docs) to get started.

Once Helm has been set up correctly, add the repo as follows:

```bash
helm repo add cipherstash https://cipherstash.github.io/proxy
```

If you had already added this repo earlier, run `helm repo update` to retrieve the latest versions of the packages. You can then run `helm search repo cipherstash` to see the charts.

To install the cipherstash-proxy chart:

```bash
helm install my-cipherstash-proxy cipherstash/cipherstash-proxy
```

To uninstall the chart:

```bash
helm uninstall my-cipherstash-proxy
```

## Configuration

### Required Configuration

The following values must be configured for the proxy to work:

```bash
helm install my-cipherstash-proxy cipherstash/cipherstash-proxy \
  --set database.host=postgres.example.com \
  --set database.name=myapp \
  --set database.username=myuser \
  --set database.password=mypass \
  --set cipherstash.workspaceCrn="your-workspace-crn" \
  --set cipherstash.clientId="your-client-id" \
  --set cipherstash.clientKey="your-client-key" \
  --set cipherstash.clientAccessKey="your-access-key"
```

| Parameter | Description | Default |
|-----------|-------------|---------|
| `database.host` | PostgreSQL database host | `replace_with_postgres_host` |
| `database.name` | PostgreSQL database name | `replace_with_postgres_database` |
| `database.port` | PostgreSQL database port | `replace_with_postgres_port` |
| `database.username` | PostgreSQL username | `replace_with_postgres_username` |
| `database.password` | PostgreSQL password | `replace_with_postgres_password` |
| `cipherstash.workspaceCrn` | CipherStash workspace CRN | `replace_with_cipherstash_workspace_crn` |
| `cipherstash.clientId` | CipherStash client ID | `replace_with_cipherstash_client_id` |
| `cipherstash.clientKey` | CipherStash client key | `replace_with_cipherstash_client_key` |
| `cipherstash.clientAccessKey` | CipherStash access key | `replace_with_cipherstash_access_key` |

**Note**: For production environments, it's recommended to use Kubernetes secrets for sensitive values instead of plain text. See the [Secrets Configuration](#secrets-configuration) section below.

### Secrets Configuration

For enhanced security, sensitive values (database password, CipherStash client key, and client access key) can be stored in Kubernetes secrets instead of plain text in values.yaml.

#### Option 1: Chart-Managed Secrets (Recommended)

The chart can create and manage secrets for you:

```yaml
secrets:
  create: true
  # Provide sensitive values here instead of in the main configuration
  databasePassword: "your-secure-database-password"
  cipherstashClientKey: "your-cipherstash-client-key"
  cipherstashClientAccessKey: "your-cipherstash-access-key"

# These values will be ignored when secrets.create is true
database:
  password: ""  # Not used when secrets are enabled
cipherstash:
  clientKey: ""  # Not used when secrets are enabled
  clientAccessKey: ""  # Not used when secrets are enabled
```

#### Option 2: External Secrets

Use existing secrets created outside of the chart:

```yaml
secrets:
  create: false
  external:
    databasePasswordSecret:
      name: "my-database-secret"
      key: "password"
    cipherstashClientKeySecret:
      name: "my-cipherstash-secret"
      key: "client-key"
    cipherstashClientAccessKeySecret:
      name: "my-cipherstash-secret"
      key: "client-access-key"
```

#### Option 3: Plain Text (Development Only)

For development environments, you can disable secrets:

```yaml
secrets:
  create: false
  # external secrets not configured
database:
  password: "dev-password"
cipherstash:
  clientKey: "dev-client-key"
  clientAccessKey: "dev-access-key"
```

| Parameter | Description | Default |
|-----------|-------------|---------|
| `secrets.create` | Whether to create secrets for sensitive values | `true` |
| `secrets.databasePassword` | Database password (when create=true) | `""` |
| `secrets.cipherstashClientKey` | CipherStash client key (when create=true) | `""` |
| `secrets.cipherstashClientAccessKey` | CipherStash client access key (when create=true) | `""` |
| `secrets.external.*.name` | Name of existing secret (when create=false) | `""` |
| `secrets.external.*.key` | Key within the secret (when create=false) | varies |

### Common Configuration Options

| Parameter | Description | Default |
|-----------|-------------|---------|
| `replicaCount` | Number of proxy replicas | `1` |
| `image.repository` | Proxy image repository | `cipherstash/proxy` |
| `image.tag` | Proxy image tag | `2.1.2` |
| `service.type` | Kubernetes service type | `ClusterIP` |
| `service.port` | Proxy service port | `6432` |
| `resources.limits.cpu` | CPU limit | `500m` |
| `resources.limits.memory` | Memory limit | `512Mi` |
| `resources.requests.cpu` | CPU request | `100m` |
| `resources.requests.memory` | Memory request | `128Mi` |

### Metrics and Monitoring

| Parameter | Description | Default |
|-----------|-------------|---------|
| `metricsService.enabled` | Enable metrics service | `true` |
| `metricsService.port` | Metrics service port | `9930` |
| `prometheus.enabled` | Enable Prometheus metrics | `true` |

### Autoscaling

| Parameter | Description | Default |
|-----------|-------------|---------|
| `autoscaling.enabled` | Enable HPA | `false` |
| `autoscaling.minReplicas` | Minimum replicas | `1` |
| `autoscaling.maxReplicas` | Maximum replicas | `100` |
| `autoscaling.targetCPUUtilizationPercentage` | Target CPU utilization | `80` |

### Ingress

| Parameter | Description | Default |
|-----------|-------------|---------|
| `ingress.enabled` | Enable ingress | `false` |
| `ingress.className` | Ingress class name | `""` |
| `ingress.annotations` | Ingress annotations | `{}` |

## Examples

### Basic Installation with Secrets (Recommended)

```yaml
# values-production.yaml
database:
  host: "postgres.production.com"
  name: "myapp"
  port: "5432"
  username: "app_user"
  # password provided via secrets

cipherstash:
  workspaceCrn: "crn:cipherstash:workspace:us-east-1:12345:workspace/my-workspace"
  clientId: "client_abc123"
  # clientKey and clientAccessKey provided via secrets

secrets:
  create: true
  databasePassword: "secure_database_password"
  cipherstashClientKey: "your_actual_client_key"
  cipherstashClientAccessKey: "your_actual_access_key"

resources:
  limits:
    cpu: 1000m
    memory: 1Gi
  requests:
    cpu: 200m
    memory: 256Mi
```

```bash
helm install cipherstash-proxy ./cipherstash-proxy-chart -f values-production.yaml
```

### Installation with External Secrets

```yaml
# values-external-secrets.yaml
database:
  host: "postgres.production.com"
  name: "myapp"
  port: "5432"
  username: "app_user"

cipherstash:
  workspaceCrn: "crn:cipherstash:workspace:us-east-1:12345:workspace/my-workspace"
  clientId: "client_abc123"

secrets:
  create: false
  external:
    databasePasswordSecret:
      name: "postgres-credentials"
      key: "password"
    cipherstashClientKeySecret:
      name: "cipherstash-credentials"
      key: "client-key"
    cipherstashClientAccessKeySecret:
      name: "cipherstash-credentials"
      key: "access-key"
```

```bash
# Create your secrets first
kubectl create secret generic postgres-credentials --from-literal=password=your_db_password
kubectl create secret generic cipherstash-credentials \
  --from-literal=client-key=your_client_key \
  --from-literal=access-key=your_access_key

# Then install the chart
helm install cipherstash-proxy ./cipherstash-proxy-chart -f values-external-secrets.yaml
```

### High Availability Setup

```yaml
# values-ha.yaml
replicaCount: 3

autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 10
  targetCPUUtilizationPercentage: 70

affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
    - weight: 100
      podAffinityTerm:
        labelSelector:
          matchExpressions:
          - key: app.kubernetes.io/name
            operator: In
            values:
            - cipherstash-proxy
        topologyKey: kubernetes.io/hostname
```

## Using the Proxy

After installation, update your application's database connection to point to the proxy:

- **Host**: `<release-name>-cipherstash-proxy.<namespace>.svc.cluster.local`
- **Port**: `6432` (or your configured service port)
- **Database, Username, Password**: Use the same credentials as your original database

## Monitoring

If metrics are enabled, Prometheus metrics are available at:
`http://<release-name>-cipherstash-proxy-metrics.<namespace>.svc.cluster.local:9930/metrics`

## Troubleshooting

### Check Proxy Logs

```bash
kubectl logs -l app.kubernetes.io/name=cipherstash-proxy -f
```

### Test Database Connectivity

```bash
kubectl exec -it deployment/<release-name>-cipherstash-proxy -- /bin/sh
# Inside the container, test connectivity to your database
```

### Verify Configuration

```bash
kubectl describe deployment <release-name>-cipherstash-proxy
```

## Upgrading

```bash
helm upgrade my-cipherstash-proxy ./cipherstash-proxy-chart
```

## Uninstalling

```bash
helm uninstall my-cipherstash-proxy
```

## Values Reference

For a complete list of configurable values, see `values.yaml` in the chart directory.

## Support

For support with CipherStash Proxy, visit [CipherStash Documentation](https://docs.cipherstash.com) or contact support@cipherstash.com. 