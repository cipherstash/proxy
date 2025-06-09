defmodule ElixirTest.Repo do
  use Ecto.Repo,
    otp_app: :elixir_test,
    adapter: Ecto.Adapters.Postgres
end
