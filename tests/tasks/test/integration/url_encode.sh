

# Taken from: https://github.com/SixArm/urlencode.sh/blob/f43155fed3b6399f3a07d974f12beb7097f9c447/urlencode.sh
urlencode() {
  local length="${#1}"
  for ((i = 0; i < length; i++)); do
    local c="${1:i:1}"
    case $c in
      [a-zA-Z0-9.~_-])
        printf '%s' "$c"
        ;;
      *)
        printf '%%%02X' "'$c"
        ;;
    esac
  done
}
