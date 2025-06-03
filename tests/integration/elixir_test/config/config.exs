import Config

config :elixir_test, ElixirTest.Repo,
  database: "cipherstash",
  username: "cipherstash",
  port: 6432,
  password: "p@ssword",
  hostname: "localhost"

config :elixir_test, ecto_repos: [ElixirTest.Repo]
