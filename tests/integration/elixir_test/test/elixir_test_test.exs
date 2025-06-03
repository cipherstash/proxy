defmodule ElixirTestTest do
  use ExUnit.Case
  doctest ElixirTest
  alias ElixirTest.Encrypted
  alias ElixirTest.Repo

  test "db connection test" do
    result = Ecto.Adapters.SQL.query!(Repo, "SELECT 1 as one")

    assert result.rows == [[1]]
  end

  test "plaintext save and load" do
    {:ok, result} =
      %Encrypted{plaintext: "plaintext content", plaintext_date: ~D[2025-06-02]}
      |> Repo.insert()

    fetched = Encrypted |> Repo.get(result.id)

    assert fetched.plaintext == "plaintext content"
    assert fetched.plaintext_date == ~D[2025-06-02]
  end

  test "encrypted text save and load" do
    {:ok, result} =
      %Encrypted{encrypted_text: "encrypted text content"}
      |> Repo.insert()

    fetched = Encrypted |> Repo.get(result.id)

    assert fetched.encrypted_text == "encrypted text content"
  end

  test "encrypted fields save and load" do
    {:ok, result} =
      %Encrypted{
        encrypted_bool: false,
        encrypted_int2: 2,
        encrypted_int4: 4,
        encrypted_int8: 8,
        encrypted_float8: 3.1415,
        encrypted_date: ~D[2025-06-01],
        encrypted_jsonb: %{top: %{array: [1, 2, 3]}}
      }
      |> Repo.insert()

    fetched = Encrypted |> Repo.get(result.id)

    assert !fetched.encrypted_bool
    assert fetched.encrypted_int2 == 2
    assert fetched.encrypted_int4 == 4
    assert fetched.encrypted_int8 == 8
    assert fetched.encrypted_float8 == 3.1415
    assert fetched.encrypted_date == ~D[2025-06-01]
    assert fetched.encrypted_jsonb == %{"top" => %{"array" => [1, 2, 3]}}
  end
end
