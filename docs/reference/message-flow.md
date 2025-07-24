# Message Handling Flow Diagrams

Below are the flow diagrams for the message handling in CipherStash Proxy for PostgreSQL requests.

### Parse

![Parse message](./images/parse.svg "Parse")


```mermaid
---
config:
  look: handDrawn
  theme: neutral
---
flowchart LR
        Parse --> P_Encryptable{Encryptable}
        P_Encryptable -->|Yes| P_MapConfig[Map column config]
        P_MapConfig --> P_Params{Has params}
        P_Encryptable -->|No| P_Write[Write]
        P_Params -->|Yes| P_RewriteParams[Rewrite params]
        P_Params -->|No| P_AddContext[Add to Context]
        P_RewriteParams --> P_AddContext
        P_AddContext --> P_Write

```

### Bind

![Bind message](./images/bind.svg "Bind")

```mermaid
---
config:
  look: handDrawn
  theme: neutral
---
flowchart LR
    Bind --> B_Context{Statement in Context}
    B_Context -->|Yes| B_Encrypt[Encrypt]
    B_Encrypt[Encrypt] --> B_RewriteParams[Rewrite params]
    B_RewriteParams --> B_Portal[Create Portal]
    B_Portal --> B_AddContext[Add to Context]
    B_Context -->|No| B_Portal
    B_AddContext --> B_Write[Write]

```




### Pipelining

Pipelining allows the client and server sides of the connection to work concurrently.
The Client sends messages without waiting for responses from the Server.
The proxy needs to keep track of Describe and Execute messages in order to know which statement or portal server messages correlate to.

The PostgreSQL server executes the queries sequentially.



```
            Sequential                              Pipelined
| Client         | Server          |    | Client         | Server          |
|----------------|-----------------|    |----------------|-----------------|
| send query 1   |                 |    | send query 1   |                 |
|                | process query 1 |    | send query 2   | process query 1 |
| receive rows 1 |                 |    | send query 3   | process query 2 |
| send query 2   |                 |    | receive rows 1 | process query 3 |
|                | process query 2 |    | receive rows 2 |                 |
| receive rows 2 |                 |    | receive rows 3 |                 |
| send query 3   |                 |
|                | process query 3 |
| receive rows 3 |                 |
```