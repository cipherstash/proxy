defmodule ElixirTest.Application do
  def start(_type, _args) do
    children = [
      ElixirTest.Repo
    ]

    opts = [strategy: :one_for_one, name: ElixirTest.Supervisor]

    Supervisor.start_link(children, opts)
  end
end
